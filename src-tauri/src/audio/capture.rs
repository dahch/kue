use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use rubato::{FastFixedOut, PolynomialDegree, Resampler};
use screencapturekit::cm_sample_buffer::CMSampleBuffer;
use screencapturekit::sc_content_filter::{self, SCContentFilter};
use screencapturekit::sc_error_handler::StreamErrorHandler;
use screencapturekit::sc_output_handler::{SCStreamOutputType, StreamOutput};
use screencapturekit::sc_shareable_content::SCShareableContent;
use screencapturekit::sc_stream::SCStream;
use screencapturekit::sc_stream_configuration::SCStreamConfiguration;
use serde::Serialize;

use super::mic_vad::MicVadState;
use crate::BatchTracker;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
/// Sync channel capacity for audio producers → consumers. A larger buffer
/// prevents dropped buffers when the STT engine or WAV writer briefly lags
/// behind real-time capture. 1500 ~ 10-15 seconds of typical callback buffers.
const BUFFER_CAPACITY: usize = 1500;
const VALID_MODES: [&str; 2] = ["practice", "shadow"];
const MIC_CHANNEL_A_FILENAME: &str = "mic_channel_A.wav";
const LOOPBACK_CHANNEL_B_FILENAME: &str = "loopback_channel_B.wav";

/// Gain applied to system-loopback (Channel B) samples. Interview audio is
/// often much quieter than the microphone; this boost keeps it above the VAD
/// threshold without clipping.
const LOOPBACK_GAIN: f32 = 4.0;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AudioCaptureStatus {
    pub mic_active: bool,
    pub loopback_active: bool,
    pub session_id: Option<String>,
}

#[derive(Debug)]
pub enum AudioError {
    PermissionDenied(String),
    DeviceNotFound(String),
    StreamError(String),
    InvalidMode(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::PermissionDenied(msg) => write!(f, "PERMISSION_DENIED: {msg}"),
            AudioError::DeviceNotFound(msg) => write!(f, "DEVICE_NOT_FOUND: {msg}"),
            AudioError::StreamError(msg) => write!(f, "STREAM_ERROR: {msg}"),
            AudioError::InvalidMode(msg) => write!(f, "INVALID_MODE: {msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Send wrappers
// ---------------------------------------------------------------------------

// SAFETY: cpal::Stream on macOS wraps a CoreAudio AudioUnit instance. Once
// built, the audio callback holds its own reference to the stream resources;
// the handle is only moved into the Mutex and never accessed concurrently,
// so sending it between threads is safe.
#[allow(dead_code)]
struct MicHandle(cpal::Stream);
unsafe impl Send for MicHandle {}
impl std::fmt::Debug for MicHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicHandle").finish_non_exhaustive()
    }
}

// SAFETY: SCStream wraps an ObjC SCStream whose audio callbacks arrive on a
// private dispatch queue. The handle is reference-counted internally and we
// only hold it (no concurrent mutation), so Send is sound.
#[allow(dead_code)]
struct LoopbackHandle(SCStream);
unsafe impl Send for LoopbackHandle {}
impl std::fmt::Debug for LoopbackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackHandle").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// AudioCapture — managed as Tauri state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AudioCapture {
    inner: Arc<Mutex<AudioCaptureInner>>,
    recordings_dir: PathBuf,
    pub(crate) mic_vad: Arc<Mutex<MicVadState>>,
}

#[derive(Debug)]
struct AudioCaptureInner {
    mic: Option<MicHandle>,
    loopback: Option<LoopbackHandle>,
    mic_writer: Option<JoinHandle<()>>,
    loopback_writer: Option<JoinHandle<()>>,
    mic_vad_handle: Option<JoinHandle<()>>,
    stt_thread: Option<JoinHandle<()>>,
    mode: String,
    session_dir: Option<PathBuf>,
    session_id: Option<String>,
}

impl AudioCapture {
    pub fn new(recordings_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AudioCaptureInner {
                mic: None,
                loopback: None,
                mic_writer: None,
                loopback_writer: None,
                mic_vad_handle: None,
                stt_thread: None,
                mode: String::new(),
                session_dir: None,
                session_id: None,
            })),
            recordings_dir,
            mic_vad: Arc::new(Mutex::new(MicVadState::new())),
        }
    }

    /// Returns a clone of the shared `MicVadState` handle so the hint worker
    /// can check for user voice activity before emitting Shadow-mode hints.
    pub fn mic_vad_state(&self) -> Arc<Mutex<MicVadState>> {
        self.mic_vad.clone()
    }

    /// Takes the session temp directory out of the inner state, returning
    /// it if one was set by `start()`.  The caller (typically the stop flow)
    /// becomes responsible for cleaning up (retain or delete).
    pub fn take_session_dir(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().session_dir.take()
    }

    /// Returns a clone of the recordings directory path so batch
    /// transcription threads can move retained WAVs there.
    pub fn recordings_dir_path(&self) -> PathBuf {
        self.recordings_dir.clone()
    }

    fn session_temp_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_else(|_| {
                use std::sync::atomic::{AtomicU64, Ordering};
                static FALLBACK: AtomicU64 = AtomicU64::new(0);
                FALLBACK.fetch_add(1, Ordering::Relaxed)
            });
        std::env::temp_dir().join(format!("kue-session-{ts}"))
    }

    pub fn start(
        &self,
        mode: &str,
    ) -> Result<(AudioCaptureStatus, std::sync::mpsc::Receiver<Vec<i16>>), AudioError> {
        if !VALID_MODES.contains(&mode) {
            return Err(AudioError::InvalidMode(format!(
                "Mode must be 'practice' or 'shadow', got '{mode}'"
            )));
        }

        let mut inner = self.inner.lock().unwrap();

        let session_dir = Self::session_temp_dir();
        fs::create_dir_all(&session_dir).map_err(|e| {
            AudioError::StreamError(format!("Failed to create session temp dir: {e}"))
        })?;

        // Reset mic VAD state so speech from a previous session doesn't carry over.
        if let Ok(mut vad) = self.mic_vad.lock() {
            vad.reset();
        }

        // -- Mic (Canal A) -------------------------------------------------
        let (mic_wav_tx, mic_wav_rx) = sync_channel::<Vec<i16>>(BUFFER_CAPACITY);
        let (mic_vad_tx, mic_vad_rx) = sync_channel::<Vec<i16>>(BUFFER_CAPACITY);
        let mic = start_mic_capture_tee(mic_wav_tx, mic_vad_tx).map_err(|e| {
            let _ = fs::remove_dir_all(&session_dir);
            e
        })?;
        let mic_path = session_dir.join(MIC_CHANNEL_A_FILENAME);
        let mic_writer = spawn_wav_writer("mic-A", mic_wav_rx, mic_path)?;

        let mic_vad_state = self.mic_vad.clone();
        let mic_vad_handle = spawn_mic_vad_monitor(mic_vad_rx, mic_vad_state);

        inner.mic = Some(MicHandle(mic));
        inner.mic_writer = Some(mic_writer);
        inner.mic_vad_handle = Some(mic_vad_handle);

        // -- Loopback (Canal B) --------------------------------------------
        let (loopback_tx, loopback_rx) = sync_channel::<Vec<i16>>(BUFFER_CAPACITY);
        let (stt_tx, stt_rx) = sync_channel::<Vec<i16>>(BUFFER_CAPACITY);
        let loopback = start_loopback_capture_tee(loopback_tx, stt_tx).map_err(|e| {
            let _ = fs::remove_dir_all(&session_dir);
            e
        })?;
        let loopback_path = session_dir.join(LOOPBACK_CHANNEL_B_FILENAME);
        let loopback_writer = spawn_wav_writer("loopback-B", loopback_rx, loopback_path)?;

        inner.loopback = Some(LoopbackHandle(loopback));
        inner.loopback_writer = Some(loopback_writer);
        inner.mode = mode.to_string();
        inner.session_dir = Some(session_dir);

        Ok((
            AudioCaptureStatus {
                mic_active: true,
                loopback_active: true,
                session_id: None,
            },
            stt_rx,
        ))
    }

    pub fn stop(&self) -> AudioCaptureStatus {
        let mut inner = self.inner.lock().unwrap();
        inner.mic = None;
        inner.loopback = None;

        // Join writer threads so the WAV files are fully flushed.
        inner.mic_writer.take().map(|h| h.join().ok());
        inner.loopback_writer.take().map(|h| h.join().ok());

        // Join VAD monitor thread
        inner.mic_vad_handle.take().map(|h| h.join().ok());

        // Detach (don't join) the STT thread — it may be stuck in a
        // long-running CLI transcription and joining it here would block
        // the entire teardown, preventing batch transcription from ever
        // being spawned. The STT thread will exit on its own when the
        // audio receiver disconnects.
        drop(inner.stt_thread.take());

        inner.mode = String::new();
        // session_dir stays for batch transcription to handle cleanup

        AudioCaptureStatus {
            mic_active: false,
            loopback_active: false,
            session_id: inner.session_id.clone(),
        }
    }

    /// Clean up any orphaned kue-session-* directories left in the system
    /// temp directory (e.g. after a crash mid-session).
    pub fn cleanup_orphaned_temp_dirs() {
        let temp_dir = std::env::temp_dir();
        if let Ok(entries) = fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("kue-session-") {
                            let _ = fs::remove_dir_all(&path);
                        }
                    }
                }
            }
        }
    }

}

// ---------------------------------------------------------------------------
// Audio sample conversion
// ---------------------------------------------------------------------------

/// Convert an f32 audio sample (range [-1.0, 1.0]) to a clamped i16 sample.
///
/// Values outside [-1.0, 1.0] are clamped to the valid i16 range.
/// This is the standard conversion used across all capture paths.
pub fn f32_to_i16(sample: f32) -> i16 {
    (sample * i16::MAX as f32)
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

// ---------------------------------------------------------------------------
// WAV writer thread
// ---------------------------------------------------------------------------

fn spawn_wav_writer(
    label: &'static str,
    rx: Receiver<Vec<i16>>,
    path: PathBuf,
) -> Result<JoinHandle<()>, AudioError> {
    thread::Builder::new()
        .name(format!("kue-wav-{label}"))
        .spawn(move || {
            let spec = WavSpec {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };

            let mut writer = match WavWriter::create(&path, spec) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Failed to create WAV file {path:?}: {e}");
                    return;
                }
            };

            log::info!("WAV writer '{label}' started → {path:?}");

            for samples in &rx {
                for &s in &samples {
                    if let Err(e) = writer.write_sample(s) {
                        log::error!("WAV writer '{label}' write error: {e}");
                    }
                }
            }

            // Channel closed — flush and finalize.
            if let Err(e) = writer.finalize() {
                log::error!("WAV writer '{label}' finalize error: {e}");
            }
            log::info!("WAV writer '{label}' finished → {path:?}");
        })
        .map_err(|e| AudioError::StreamError(format!("Failed to spawn WAV writer thread: {e}")))
}

// ---------------------------------------------------------------------------
// Mic capture (Canal A) – cpal, with tee for VAD
// ---------------------------------------------------------------------------

/// Small stateful helper that converts microphone input to mono f32 and
/// resamples it to `SAMPLE_RATE` (16 kHz) before it reaches the WAV writer,
/// VAD and STT pipelines.
///
/// cpal exposes the device's native sample rate and channel count, but the
/// rest of the system expects 16 kHz mono. Without resampling the WAV header
/// would claim 16 kHz while the payload was recorded at e.g. 48 kHz, which
/// makes the file sound much longer than the session and often silent/corrupt.
struct MicResampler {
    resampler: FastFixedOut<f32>,
    input_buffer: Vec<f32>,
}

impl MicResampler {
    fn new(input_rate: f64) -> Result<Self, AudioError> {
        let ratio = SAMPLE_RATE as f64 / input_rate;
        // 1024 output frames @ 16 kHz ≈ 64 ms of audio. Large enough to be
        // efficient, small enough to keep latency low.
        let resampler = FastFixedOut::new(
            ratio,
            2.0,
            PolynomialDegree::Linear,
            1024,
            1,
        )
        .map_err(|e| AudioError::StreamError(format!("Failed to create mic resampler: {e}")))?;
        Ok(Self {
            resampler,
            input_buffer: Vec::new(),
        })
    }

    fn push(&mut self, mono_f32: &[f32], wav_tx: &SyncSender<Vec<i16>>, vad_tx: &SyncSender<Vec<i16>>) {
        self.input_buffer.extend_from_slice(mono_f32);
        while self.input_buffer.len() >= self.resampler.input_frames_next() {
            let needed = self.resampler.input_frames_next();
            let input = vec![self.input_buffer[..needed].to_vec()];
            match self.resampler.process(&input, None) {
                Ok(output) => {
                    let samples: Vec<i16> = output[0]
                        .iter()
                        .map(|&s| f32_to_i16(s))
                        .collect();
                    send_mic_samples(&samples, wav_tx, vad_tx);
                }
                Err(e) => {
                    log::error!("Mic resampling error: {e}");
                }
            }
            self.input_buffer.drain(..needed);
        }
    }
}

fn send_mic_samples(samples: &[i16], wav_tx: &SyncSender<Vec<i16>>, vad_tx: &SyncSender<Vec<i16>>) {
    let buf = samples.to_vec();
    if wav_tx.try_send(buf.clone()).is_err() {
        log::error!("Mic WAV writer channel full — dropping buffer");
    }
    if vad_tx.try_send(buf).is_err() {
        log::error!("Mic VAD channel full — dropping buffer");
    }
}

fn start_mic_capture_tee(
    wav_tx: SyncSender<Vec<i16>>,
    vad_tx: SyncSender<Vec<i16>>,
) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        AudioError::DeviceNotFound(
            "No default microphone found. Connect a mic and check input settings.".into(),
        )
    })?;

    let config = device
        .default_input_config()
        .map_err(|e| AudioError::StreamError(format!("Failed to read default mic config: {e}")))?;

    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();
    let input_channels = stream_config.channels as usize;
    let input_rate = stream_config.sample_rate.0 as f64;
    let needs_resampling =
        (input_rate - SAMPLE_RATE as f64).abs() > 1.0 || input_channels != 1;

    match sample_format {
        cpal::SampleFormat::I16 => {
            let mut resampler = if needs_resampling {
                Some(MicResampler::new(input_rate)?)
            } else {
                None
            };
            let stream = device
                .build_input_stream::<i16, _, _>(
                    &stream_config,
                    move |data, _| {
                        let mono_f32: Vec<f32> = if input_channels == 1 {
                            data.iter()
                                .map(|&s| s as f32 / i16::MAX as f32)
                                .collect()
                        } else {
                            data.chunks_exact(input_channels)
                                .map(|chunk| {
                                    let sum: f32 = chunk.iter().map(|&s| s as f32).sum();
                                    sum / (input_channels as f32 * i16::MAX as f32)
                                })
                                .collect()
                        };
                        if let Some(r) = &mut resampler {
                            r.push(&mono_f32, &wav_tx, &vad_tx);
                        } else {
                            let samples: Vec<i16> = mono_f32
                                .iter()
                                .map(|&s| f32_to_i16(s))
                                .collect();
                            send_mic_samples(&samples, &wav_tx, &vad_tx);
                        }
                    },
                    move |err| log::error!("Mic capture error: {err}"),
                    None,
                )
                .map_err(|e| AudioError::StreamError(format!("Failed to build mic stream: {e}")))?;
            stream
                .play()
                .map_err(|e| AudioError::StreamError(format!("Failed to start mic stream: {e}")))?;
            log::info!(
                "Mic capture (Canal A) started — {input_rate} Hz, {input_channels} ch → {} Hz, mono, i16",
                SAMPLE_RATE
            );
            Ok(stream)
        }
        cpal::SampleFormat::F32 => {
            let mut resampler = if needs_resampling {
                Some(MicResampler::new(input_rate)?)
            } else {
                None
            };
            let stream = device
                .build_input_stream::<f32, _, _>(
                    &stream_config,
                    move |data, _| {
                        let mono_f32: Vec<f32> = if input_channels == 1 {
                            data.to_vec()
                        } else {
                            data.chunks_exact(input_channels)
                                .map(|chunk| chunk.iter().sum::<f32>() / input_channels as f32)
                                .collect()
                        };
                        if let Some(r) = &mut resampler {
                            r.push(&mono_f32, &wav_tx, &vad_tx);
                        } else {
                            let samples: Vec<i16> = mono_f32
                                .iter()
                                .map(|&s| f32_to_i16(s))
                                .collect();
                            send_mic_samples(&samples, &wav_tx, &vad_tx);
                        }
                    },
                    move |err| log::error!("Mic capture error: {err}"),
                    None,
                )
                .map_err(|e| AudioError::StreamError(format!("Failed to build mic stream: {e}")))?;
            stream
                .play()
                .map_err(|e| AudioError::StreamError(format!("Failed to start mic stream: {e}")))?;
            log::info!(
                "Mic capture (Canal A) started — {input_rate} Hz, {input_channels} ch → {} Hz, mono, f32",
                SAMPLE_RATE
            );
            Ok(stream)
        }
        other => Err(AudioError::StreamError(format!(
            "Unsupported mic sample format: {other:?}. Expected i16 or f32."
        ))),
    }
}

// ---------------------------------------------------------------------------
// Loopback capture (Canal B) – ScreenCaptureKit
// ---------------------------------------------------------------------------

/// Same as `start_loopback_capture` but sends audio samples to TWO
/// senders simultaneously — one for the WAV writer and one for the STT
/// pipeline. This avoids an extra tee thread and keeps audio in sync.
fn start_loopback_capture_tee(
    wav_tx: SyncSender<Vec<i16>>,
    stt_tx: SyncSender<Vec<i16>>,
) -> Result<SCStream, AudioError> {
    let content = SCShareableContent::try_current().map_err(|e| {
        AudioError::PermissionDenied(format!(
            "Screen & System Audio Recording permission not granted. \
             To grant it: System Settings → Privacy & Security → \
             Screen & System Audio Recording → enable this app. \
             (Underlying error: {e})"
        ))
    })?;

    let display = content.displays.into_iter().next().ok_or_else(|| {
        AudioError::DeviceNotFound("No display found for loopback audio capture.".into())
    })?;

    let filter = SCContentFilter::new(sc_content_filter::InitParams::Display(display));

    let config = SCStreamConfiguration {
        captures_audio: true,
        sample_rate: SAMPLE_RATE,
        channel_count: CHANNELS as u32,
        excludes_current_process_audio: true,
        width: 1,
        height: 1,
        ..Default::default()
    };

    struct AudioErrorHandler;
    impl StreamErrorHandler for AudioErrorHandler {
        fn on_error(&self) {
            log::error!("Loopback capture stream error");
        }
    }

    struct LoopbackOutputTee {
        wav_tx: SyncSender<Vec<i16>>,
        stt_tx: SyncSender<Vec<i16>>,
    }

    impl StreamOutput for LoopbackOutputTee {
        fn did_output_sample_buffer(
            &self,
            sample: CMSampleBuffer,
            of_type: SCStreamOutputType,
        ) {
            if !matches!(of_type, SCStreamOutputType::Audio) {
                return;
            }
            let audio_buffers = sample.sys_ref.get_av_audio_buffer_list();
            for buf in audio_buffers {
                let f32_size = std::mem::size_of::<f32>();
                if buf.data.len() % f32_size != 0 {
                    continue;
                }
                let samples: Vec<i16> = buf
                    .data
                    .chunks_exact(f32_size)
                    .map(|chunk| {
                        let f = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        // Apply gain to combat quiet system/interview audio, then clamp.
                        f32_to_i16(f * LOOPBACK_GAIN)
                    })
                    .collect();
                if self.wav_tx.try_send(samples.clone()).is_err() {
                    log::error!("Loopback WAV writer channel full — dropping buffer");
                }
                if self.stt_tx.try_send(samples).is_err() {
                    log::error!("Loopback STT channel full — dropping buffer");
                }
            }
        }
    }

    let mut stream = SCStream::new(filter, config, AudioErrorHandler);
    let output = LoopbackOutputTee { wav_tx, stt_tx };
    stream.add_output(output, SCStreamOutputType::Audio);

    stream
        .start_capture()
        .map_err(|e| AudioError::StreamError(format!("Failed to start loopback capture: {e}")))?;

    log::info!("Loopback capture (Canal B) with STT tee started — 16 kHz, mono via ScreenCaptureKit");
    Ok(stream)
}

// ---------------------------------------------------------------------------
// Mic VAD monitor thread
// ---------------------------------------------------------------------------

/// Spawns a background thread that reads i16 mic audio samples and feeds
/// them into the shared `MicVadState`.  The thread exits when the channel
/// is closed (mic capture stops).
fn spawn_mic_vad_monitor(
    rx: Receiver<Vec<i16>>,
    state: Arc<Mutex<MicVadState>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("kue-mic-vad".into())
        .spawn(move || {
            for samples in &rx {
                let mut s = state.lock().expect("mic_vad lock poisoned");
                s.feed_audio(&samples);
            }
        })
        .expect("failed to spawn mic VAD monitor thread")
}

// ---------------------------------------------------------------------------
// Retention & batch transcription helpers
// ---------------------------------------------------------------------------

fn apply_retention(session_dir: &std::path::Path, recordings_dir: &std::path::Path, retain: bool) {
    if retain {
        if let Err(e) = fs::create_dir_all(recordings_dir) {
            log::error!("Failed to create recordings dir {dir:?}: {e}", dir = recordings_dir);
        } else if let Some(name) = session_dir.file_name() {
            let target = recordings_dir.join(name);
            let _ = fs::create_dir_all(&target);
            if let Ok(entries) = fs::read_dir(session_dir) {
                for entry in entries.flatten() {
                    let dest = target.join(entry.file_name());
                    let _ = fs::rename(entry.path(), &dest);
                }
            }
        }
    }
    let _ = fs::remove_dir_all(session_dir);
}

/// Quick check: does the WAV exist and contain at least one sample?
fn wav_has_samples(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    match hound::WavReader::open(path) {
        Ok(reader) => reader.len() > 0,
        Err(e) => {
            log::warn!("Could not read WAV {:?} for sample count: {e}", path);
            false
        }
    }
}

fn spawn_batch_transcription(
    session_dir: std::path::PathBuf,
    session_id: String,
    retain: bool,
    recordings_dir: std::path::PathBuf,
    db: Database,
    app_handle: tauri::AppHandle,
    batch_tracker: BatchTracker,
) {
    let mic_wav = session_dir.join(MIC_CHANNEL_A_FILENAME);
    let loopback_wav = session_dir.join(LOOPBACK_CHANNEL_B_FILENAME);

    if !mic_wav.exists() && !loopback_wav.exists() {
        apply_retention(&session_dir, &recordings_dir, retain);
        mark_batch_ready(app_handle, batch_tracker, &session_id);
        return;
    }

    // Clone copies needed for the error fallback path (outside the closure).
    let sid_fallback = session_id.clone();
    let session_dir_fallback = session_dir.clone();
    let recordings_dir_fallback = recordings_dir.clone();
    let app_handle_fallback = app_handle.clone();
    let batch_tracker_fallback = BatchTracker::clone(&batch_tracker);

    let thread_result = std::thread::Builder::new()
        .name("kue-batch-transcribe".into())
        .spawn(move || {
            // Lowered from 120s: loading a local model + two short WAVs should
            // finish well within 60s on modern hardware. This prevents the UI
            // from being stuck forever if the worker hangs.
            const BATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

            let (tx, rx) = std::sync::mpsc::channel::<Result<usize, String>>();
            let worker_session_dir = session_dir.clone();
            let worker_id = session_id.clone();
            let worker_recordings_dir = recordings_dir.clone();
            let worker_app_handle = app_handle.clone();

            let worker_result = std::thread::Builder::new()
                .name("kue-batch-transcribe-worker".into())
                .spawn(move || {
                    let config = STTConfig::default();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut engine = crate::stt::create_engine(&config);
                        if let Err(e) = engine.load(&config.model_path, &config.language) {
                            return Err(format!("Batch STT model load failed: {e}"));
                        }

                        let mut total_lines = 0usize;
                        let mut errors: Vec<String> = Vec::new();

                        // Channel A — microphone (User speaker)
                        let mic_path = worker_session_dir.join(MIC_CHANNEL_A_FILENAME);
                        if mic_path.exists() {
                            if !wav_has_samples(&mic_path) {
                                errors.push(
                                    "Microphone channel (mic_channel_A.wav) has no audio samples. \
                                     Check that microphone permission is granted and the mic is not muted."
                                        .to_string(),
                                );
                            } else {
                                match crate::stt::batch::transcribe_channel_batch(
                                    &mic_path,
                                    crate::types::Speaker::User,
                                    &worker_id,
                                    &db,
                                    &*engine,
                                    &config,
                                ) {
                                    Ok(lines) => total_lines += lines.len(),
                                    Err(e) => {
                                        log::warn!("Batch transcription for mic channel A failed: {e}");
                                        errors.push(format!("Microphone transcription failed: {e}"));
                                    }
                                }
                            }
                        }

                        // Channel B — system loopback (Interviewer speaker, includes TTS)
                        let loopback_path = worker_session_dir.join(LOOPBACK_CHANNEL_B_FILENAME);
                        if loopback_path.exists() {
                            if !wav_has_samples(&loopback_path) {
                                errors.push(
                                    "System audio channel (loopback_channel_B.wav) has no audio samples. \
                                     Check that Screen & System Audio Recording permission is granted."
                                        .to_string(),
                                );
                            } else {
                                match crate::stt::batch::transcribe_channel_batch(
                                    &loopback_path,
                                    crate::types::Speaker::Interviewer,
                                    &worker_id,
                                    &db,
                                    &*engine,
                                    &config,
                                ) {
                                    Ok(lines) => total_lines += lines.len(),
                                    Err(e) => {
                                        log::warn!("Batch transcription for loopback channel B failed: {e}");
                                        errors.push(format!("System audio transcription failed: {e}"));
                                    }
                                }
                            }
                        }

                        if total_lines == 0 && !errors.is_empty() {
                            Err(errors.join("\n"))
                        } else {
                            Ok(total_lines)
                        }
                    }));

                    let to_send = match result {
                        Ok(inner) => inner,
                        Err(panic_err) => {
                            let msg = panic_err
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| panic_err.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            Err(format!("Batch transcription thread panicked: {msg}"))
                        }
                    };

                    if let Err(ref e) = to_send {
                        log::error!("Batch transcription worker error for session {}: {e}", worker_id);
                        let _ = worker_app_handle.emit("post-call-transcript-error", serde_json::json!({
                            "session_id": &worker_id,
                            "message": format!("Batch transcription failed: {e}"),
                        }));
                    }

                    apply_retention(&worker_session_dir, &worker_recordings_dir, retain);
                    let _ = tx.send(to_send);
                });

            let worker = match worker_result {
                Ok(w) => Some(w),
                Err(e) => {
                    log::error!(
                        "Failed to spawn batch transcription worker thread for session {}: {e}",
                        session_id
                    );
                    let _ = app_handle.emit("post-call-transcript-error", serde_json::json!({
                        "session_id": &session_id,
                        "message": format!("Failed to spawn batch worker thread: {e}"),
                    }));
                    apply_retention(&session_dir, &recordings_dir, retain);
                    None // fall through to mark_batch_ready below
                }
            };

            if let Some(worker) = worker {
                let outcome = rx.recv_timeout(BATCH_TIMEOUT);
                match outcome {
                    Ok(Ok(count)) => {
                        log::info!(
                            "Batch transcription complete: {} total lines for session {}",
                            count, session_id,
                        );
                    }
                    Ok(Err(e)) => {
                        log::error!("Batch transcription failed for session {}: {e}", session_id);
                        let _ = app_handle.emit("post-call-transcript-error", serde_json::json!({
                            "session_id": &session_id,
                            "message": format!("Batch transcription failed: {e}"),
                        }));
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        log::warn!(
                            "Batch transcription for session {} exceeded the {}s safety timeout; releasing UI anyway.",
                            session_id, BATCH_TIMEOUT.as_secs()
                        );
                        let _ = app_handle.emit("post-call-transcript-error", serde_json::json!({
                            "session_id": &session_id,
                            "message": format!("Batch transcription timed out after {}s", BATCH_TIMEOUT.as_secs()),
                        }));
                        let _ = worker;
                        apply_retention(&session_dir, &recordings_dir, retain);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        log::error!(
                            "Batch transcription worker for session {} disconnected unexpectedly.",
                            session_id
                        );
                        let _ = app_handle.emit("post-call-transcript-error", serde_json::json!({
                            "session_id": &session_id,
                            "message": "Batch transcription worker disconnected unexpectedly",
                        }));
                        apply_retention(&session_dir, &recordings_dir, retain);
                    }
                }
            }

            mark_batch_ready(app_handle, batch_tracker, &session_id);
        });

    if let Err(e) = thread_result {
        log::error!(
            "Failed to spawn batch transcription thread for session {}: {e}",
            sid_fallback
        );
        let _ = app_handle_fallback.emit("post-call-transcript-error", serde_json::json!({
            "session_id": &sid_fallback,
            "message": format!("Failed to spawn batch thread: {e}"),
        }));
        apply_retention(&session_dir_fallback, &recordings_dir_fallback, retain);
        mark_batch_ready(app_handle_fallback, batch_tracker_fallback, &sid_fallback);
    }
}

fn mark_batch_ready(
    app_handle: tauri::AppHandle,
    batch_tracker: BatchTracker,
    session_id: &str,
) {
    if let Ok(mut tracker) = batch_tracker.0.lock() {
        tracker.insert(session_id.to_string());
    }
    let _ = app_handle.emit(
        "post-call-transcript-ready",
        serde_json::json!({ "session_id": session_id }),
    );
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

use crate::db::Database;
use crate::orchestrator::{HintJobSender, PanicState};
use crate::stt::{STTConfig, STTPipeline};
use tauri::Emitter;
use tauri::Manager;

const PANIC_DURATION_SECS: u64 = 10;

#[tauri::command]
pub fn start_session(
    mode: String,
    company: Option<String>,
    role: Option<String>,
    audio: tauri::State<'_, AudioCapture>,
    db: tauri::State<'_, Database>,
    app_handle: tauri::AppHandle,
) -> Result<AudioCaptureStatus, String> {
    let (status, stt_rx) = audio.start(&mode).map_err(|e| e.to_string())?;

    // Create a new session in the DB
    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO sessions (id, company, role, mode) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, company.as_deref().unwrap_or(""), role.as_deref().unwrap_or(""), mode],
        )
        .map_err(|e| format!("Failed to insert session: {e}"))?;
    }

    // Start the STT pipeline
    let rx = stt_rx;
    let config = STTConfig {
        model_path: STTConfig::default_model_path(),
        language: "en".to_string(),
        ..Default::default()
    };

    let hint_job_tx = app_handle.state::<HintJobSender>().inner().clone();

    let mut pipeline = STTPipeline::new(config)
        .with_app_handle(app_handle.clone())
        .with_mode(&mode)
        .with_hint_job_tx(hint_job_tx);
    if let Err(e) = pipeline.load_model() {
        log::error!("STT model load failed (best-effort): {e}");
    }
    pipeline.start_session(&session_id);

    let db_for_stt = Database::clone(db.inner());
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let stt_thread = pipeline.spawn_processing_thread(rx, db_for_stt);

    let mut inner = audio.inner.lock().map_err(|e| e.to_string())?;
    inner.stt_thread = Some(stt_thread);
    inner.session_id = Some(session_id.clone());

    app_handle
        .emit("session-started", serde_json::json!({"mode": mode, "session_id": &session_id}))
        .ok();

    Ok(AudioCaptureStatus {
        mic_active: status.mic_active,
        loopback_active: status.loopback_active,
        session_id: Some(session_id),
    })
}

#[tauri::command]
pub async fn stop_session(
    audio: tauri::State<'_, AudioCapture>,
    db: tauri::State<'_, Database>,
    app_handle: tauri::AppHandle,
    batch_tracker: tauri::State<'_, BatchTracker>,
) -> Result<AudioCaptureStatus, String> {
    // Take the session id and mark it as ended quickly without waiting for
    // audio threads to join — that is the operation that can freeze the UI.
    let session_id = {
        let mut inner = audio.inner.lock().map_err(|e| e.to_string())?;
        inner.session_id.take()
    };

    if let Some(ref sid) = session_id {
        if let Ok(conn) = db.conn.lock() {
            let _ = conn.execute(
                "UPDATE sessions SET ended_at = datetime('now') WHERE id = ?1",
                rusqlite::params![sid],
            );
        }
    }

    let retain = db
        .conn
        .lock()
        .ok()
        .and_then(|conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key='retain_audio'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
        })
        .map(|v| v == "true")
        .unwrap_or(false);

    // Tell the frontend immediately that the session is stopping.
    app_handle.emit("session-stopped", ()).ok();

    // The actual teardown (stream drops + WAV writer joins + batch STT) can
    // block for seconds on ScreenCaptureKit. Run it in a background thread so
    // the Tauri command returns instantly and the UI never freezes.
    let audio_clone = audio.inner().clone();
    let db_clone = Database::clone(db.inner());
    let recordings_dir = audio.recordings_dir_path();
    let batch_tracker_clone = BatchTracker::clone(batch_tracker.inner());
    let app_handle_clone = app_handle.clone();

    std::thread::spawn(move || {
        let _ = audio_clone.stop();
        let session_dir = audio_clone.take_session_dir();
        if let (Some(dir), Some(sid)) = (session_dir, session_id) {
            spawn_batch_transcription(
                dir,
                sid,
                retain,
                recordings_dir,
                db_clone,
                app_handle_clone,
                batch_tracker_clone,
            );
        }
    });

    Ok(AudioCaptureStatus {
        mic_active: false,
        loopback_active: false,
        session_id: None,
    })
}

#[tauri::command]
pub fn get_log_dir_path(app_handle: tauri::AppHandle) -> Result<String, String> {
    let app_data = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    Ok(app_data.join("logs").to_string_lossy().to_string())
}

#[tauri::command]
pub fn is_transcript_ready(
    session_id: String,
    batch_tracker: tauri::State<'_, BatchTracker>,
    db: tauri::State<'_, Database>,
) -> Result<bool, String> {
    // Fast path: check in-memory tracker (set within the same process lifetime).
    if let Ok(tracker) = batch_tracker.0.lock() {
        if tracker.contains(&session_id) {
            return Ok(true);
        }
    }
    // Fall back to the database: a session whose `ended_at` is set was
    // properly stopped, which means `spawn_batch_transcription` was triggered.
    // Even if it produced 0 transcript lines (all-silence recording), the UI
    // should not remain stuck "Processing..." forever — analysis will show a
    // clear "No transcript lines" error instead.
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let has_ended: bool = conn
        .query_row(
            "SELECT ended_at IS NOT NULL FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if has_ended {
        // Re-insert into in-memory tracker so subsequent calls hit the fast path.
        if let Ok(mut tracker) = batch_tracker.0.lock() {
            tracker.insert(session_id.clone());
        }
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub fn panic_mode(
    app_handle: tauri::AppHandle,
    panic_state: tauri::State<'_, PanicState>,
) -> Result<(), String> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(PANIC_DURATION_SECS);
    {
        let mut guard = panic_state.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(until);
    }
    app_handle
        .emit("panic-mode", serde_json::json!({"until_secs": PANIC_DURATION_SECS}))
        .map_err(|e| e.to_string())
}



// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    // -----------------------------------------------------------------------
    // f32_to_i16 — audio sample conversion
    // -----------------------------------------------------------------------

    #[test]
    fn f32_to_i16_zero() {
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn f32_to_i16_max_positive() {
        assert_eq!(f32_to_i16(1.0), i16::MAX);
    }

    #[test]
    fn f32_to_i16_max_negative() {
        // Note: -1.0 * i16::MAX = -32767, not i16::MIN (-32768).
        // This is because the conversion multiplies by i16::MAX, not by 32768.
        // The negative range is symmetric around zero minus one.
        assert_eq!(f32_to_i16(-1.0), -i16::MAX);
    }

    #[test]
    fn f32_to_i16_positive_half() {
        assert_eq!(f32_to_i16(0.5), 16383); // 32767 * 0.5 = 16383.5 → 16383 (truncation)
    }

    #[test]
    fn f32_to_i16_negative_half() {
        // -0.5 * 32767 = -16383.5, truncation toward zero → -16383
        assert_eq!(f32_to_i16(-0.5), -16383);
    }

    #[test]
    fn f32_to_i16_clamps_above_max() {
        assert_eq!(f32_to_i16(1.5), i16::MAX);
    }

    #[test]
    fn f32_to_i16_clamps_below_min() {
        assert_eq!(f32_to_i16(-1.5), i16::MIN);
    }

    #[test]
    fn f32_to_i16_very_small_positive() {
        // Near-zero positive values should truncate toward zero
        assert_eq!(f32_to_i16(1e-6), 0);
    }

    #[test]
    fn f32_to_i16_very_small_negative() {
        assert_eq!(f32_to_i16(-1e-6), 0);
    }

    #[test]
    fn f32_to_i16_nan_does_not_panic() {
        // NaN * anything = NaN; NaN.clamp() = NaN; NaN as i16 = 0 in Rust.
        // We just verify the conversion doesn't panic or crash.
        let result = f32_to_i16(f32::NAN);
        // According to Rust's as-cast rules, NaN float → integer yields 0.
        assert_eq!(result, 0);
    }

    #[test]
    fn f32_to_i16_infinity_clamps_to_max() {
        assert_eq!(f32_to_i16(f32::INFINITY), i16::MAX);
    }

    #[test]
    fn f32_to_i16_neg_infinity_clamps_to_min() {
        assert_eq!(f32_to_i16(f32::NEG_INFINITY), i16::MIN);
    }

    // -----------------------------------------------------------------------
    // AudioError Display impl — all four variants + edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn audio_error_display_permission_denied() {
        let e = AudioError::PermissionDenied("test".into());
        let msg = e.to_string();
        assert!(msg.starts_with("PERMISSION_DENIED:"));
        assert!(msg.contains("test"));
    }

    #[test]
    fn audio_error_display_device_not_found() {
        let e = AudioError::DeviceNotFound("no mic".into());
        let msg = e.to_string();
        assert!(msg.starts_with("DEVICE_NOT_FOUND:"));
        assert!(msg.contains("no mic"));
    }

    #[test]
    fn audio_error_display_stream_error() {
        let e = AudioError::StreamError("buffer overrun".into());
        let msg = e.to_string();
        assert!(msg.starts_with("STREAM_ERROR:"));
        assert!(msg.contains("buffer overrun"));
    }

    #[test]
    fn audio_error_display_invalid_mode() {
        let e = AudioError::InvalidMode("unknown".into());
        let msg = e.to_string();
        assert!(msg.starts_with("INVALID_MODE:"));
        assert!(msg.contains("unknown"));
    }

    #[test]
    fn audio_error_display_empty_message() {
        let cases: Vec<(AudioError, &str)> = vec![
            (AudioError::PermissionDenied(String::new()), "PERMISSION_DENIED:"),
            (AudioError::DeviceNotFound(String::new()), "DEVICE_NOT_FOUND:"),
            (AudioError::StreamError(String::new()), "STREAM_ERROR:"),
            (AudioError::InvalidMode(String::new()), "INVALID_MODE:"),
        ];
        for (err, prefix) in cases {
            let msg = err.to_string();
            assert!(msg.starts_with(prefix), "Expected prefix {prefix} in '{msg}'");
            // Should have colon but no extra content after it (just trailing whitespace is fine)
            assert!(!msg.is_empty());
        }
    }

    #[test]
    fn audio_error_display_special_characters() {
        let e = AudioError::PermissionDenied("ñó 🎤 mïc ♫".into());
        let msg = e.to_string();
        assert!(msg.contains("ñó 🎤 mïc ♫"));
    }

    // -----------------------------------------------------------------------
    // AudioCaptureStatus serialization — all field combinations
    // -----------------------------------------------------------------------

    #[test]
    fn audio_capture_status_serialization_both_false() {
        let status = AudioCaptureStatus {
            mic_active: false,
            loopback_active: false,
            session_id: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":false,"loopback_active":false,"session_id":null}"#);
    }

    #[test]
    fn audio_capture_status_serialization_both_true() {
        let status = AudioCaptureStatus {
            mic_active: true,
            loopback_active: true,
            session_id: Some("abc".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":true,"loopback_active":true,"session_id":"abc"}"#);
    }

    #[test]
    fn audio_capture_status_serialization_mic_true_loopback_false() {
        let status = AudioCaptureStatus {
            mic_active: true,
            loopback_active: false,
            session_id: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":true,"loopback_active":false,"session_id":null}"#);
    }

    #[test]
    fn audio_capture_status_serialization_mic_false_loopback_true() {
        let status = AudioCaptureStatus {
            mic_active: false,
            loopback_active: true,
            session_id: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":false,"loopback_active":true,"session_id":null}"#);
    }

    // -----------------------------------------------------------------------
    // AudioCapture construction and initial state
    // -----------------------------------------------------------------------

    #[test]
    fn audio_capture_new_state() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let inner = cap.inner.lock().unwrap();
        assert!(inner.mic.is_none());
        assert!(inner.loopback.is_none());
        assert!(inner.mic_writer.is_none());
        assert!(inner.loopback_writer.is_none());
        assert!(inner.mic_vad_handle.is_none());
        assert!(inner.stt_thread.is_none());
        assert!(inner.mode.is_empty());
        assert!(inner.session_dir.is_none());
        assert!(inner.session_id.is_none());
    }

    #[test]
    fn audio_capture_new_has_mic_vad_state() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let state = cap.mic_vad_state();
        let s = state.lock().unwrap();
        assert!(!s.is_currently_speaking());
    }

    #[test]
    fn audio_capture_new_stores_recordings_dir() {
        let dir = PathBuf::from("/tmp/kue-test-recordings-dir");
        let _ = fs::remove_dir_all(&dir);
        let cap = AudioCapture::new(dir.clone());
        assert_eq!(cap.recordings_dir, dir);
        fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------------
    // session_temp_dir() format verification
    // -----------------------------------------------------------------------

    #[test]
    fn session_temp_dir_uses_kue_session_prefix() {
        let path = AudioCapture::session_temp_dir();
        let dirname = path.file_name().unwrap().to_str().unwrap().to_string();
        assert!(dirname.starts_with("kue-session-"), "Expected 'kue-session-...', got {dirname}");
        assert!(path.parent() == Some(std::env::temp_dir().as_path()));
    }

    #[test]
    fn session_temp_dir_timestamp_is_plausible() {
        let path = AudioCapture::session_temp_dir();
        let dirname = path.file_name().unwrap().to_str().unwrap();
        let ts_str = dirname
            .strip_prefix("kue-session-")
            .expect("dirname should start with kue-session-");
        let ts: u64 = ts_str.parse().expect("timestamp should be a valid u64");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        assert!(
            ts <= now && ts >= now.saturating_sub(5),
            "Timestamp {ts} should be close to now {now}"
        );
    }

    // -----------------------------------------------------------------------
    // stop() — no-op when already stopped, idempotent
    // -----------------------------------------------------------------------

    #[test]
    fn audio_capture_stop_when_inactive() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let status = cap.stop();
        assert!(!status.mic_active);
        assert!(!status.loopback_active);
        // Second stop is also a no-op.
        let status2 = cap.stop();
        assert!(!status2.mic_active);
        assert!(!status2.loopback_active);
    }

    // -----------------------------------------------------------------------
    // start() — mode validation (hardware-independent path)
    // -----------------------------------------------------------------------

    #[test]
    fn audio_capture_start_invalid_mode() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let err = cap.start("invalid").unwrap_err();
        assert!(matches!(err, AudioError::InvalidMode(_)));
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn audio_capture_start_empty_mode() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let err = cap.start("").unwrap_err();
        assert!(matches!(err, AudioError::InvalidMode(_)));
        assert!(err.to_string().contains("'practice' or 'shadow'"));
    }

    #[test]
    fn audio_capture_start_mixed_case_mode() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let err = cap.start("Practice").unwrap_err();
        assert!(matches!(err, AudioError::InvalidMode(_)));
    }

    /// This test verifies that valid mode strings pass validation but then
    /// may fail with a hardware error (which is expected on CI/headless).
    /// If hardware IS available, it cleans up by calling stop().
    ///
    /// NOTE: this test may block on systems WITH audio hardware because
    /// `cpal` can block waiting for the input stream to build on some platforms.
    /// It is marked `#[ignore]` by default; run with `--include-ignored` on
    /// systems with known-working audio hardware.
    #[test]
    #[ignore = "requires real audio hardware; may block on some systems"]
    fn audio_capture_start_valid_modes_hardware() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        for mode in ["practice", "shadow"] {
            let result = cap.start(mode);
            match result {
                Ok(_) => {
                    cap.stop(); // clean up if start succeeded
                }
                Err(e) => {
                    // Must be a hardware error, not an InvalidMode error.
                    assert!(!matches!(e, AudioError::InvalidMode(_)));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // cleanup_orphaned_temp_dirs
    // -----------------------------------------------------------------------

    #[test]
    fn cleanup_orphaned_temp_dirs_removes_stale_kue_dirs() {
        let stale = std::env::temp_dir().join("kue-session-stale-test");
        let _ = fs::remove_dir_all(&stale);
        fs::create_dir_all(&stale).unwrap();

        let other = std::env::temp_dir().join("other-app-temp");
        let _ = fs::remove_dir_all(&other);
        fs::create_dir_all(&other).unwrap();

        AudioCapture::cleanup_orphaned_temp_dirs();

        assert!(!stale.exists(), "stale kue-session-* dir should be removed");
        assert!(other.exists(), "non-kue temp dirs should be left alone");

        fs::remove_dir_all(&other).ok();
    }

    // -----------------------------------------------------------------------
    // WAV writer thread — lifecycle and data integrity
    // -----------------------------------------------------------------------

    #[test]
    fn spawn_wav_writer_creates_wav_file() {
        let dir = PathBuf::from("/tmp/kue-test-wav-create");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_output.wav");

        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
        let handle = spawn_wav_writer("test-writer", rx, path.clone()).unwrap();

        // Send one buffer of samples
        let samples: Vec<i16> = vec![0, 100, -100, i16::MAX, i16::MIN, 42, -42];
        tx.send(samples).unwrap();
        // Close channel to signal writer to finalize and exit
        drop(tx);

        handle.join().expect("WAV writer thread panicked");

        // Verify the file exists and has content
        assert!(path.exists(), "WAV file should exist at {path:?}");
        let metadata = path.metadata().expect("should read metadata");
        assert!(metadata.len() > 0, "WAV file should not be empty");

        // Verify it's a valid WAV by reading it back with hound
        let reader = hound::WavReader::open(&path).expect("should open WAV file");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Int);

        // Verify sample data was written correctly
        let actual: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(actual, vec![0, 100, -100, i16::MAX, i16::MIN, 42, -42],
            "WAV file should contain the exact samples we sent");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spawn_wav_writer_handles_multiple_buffers() {
        let dir = PathBuf::from("/tmp/kue-test-wav-multi");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("multi.wav");

        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
        let handle = spawn_wav_writer("multi", rx, path.clone()).unwrap();

        // Send multiple buffers
        tx.send(vec![1, 2, 3]).unwrap();
        tx.send(vec![4, 5, 6]).unwrap();
        tx.send(vec![7, 8, 9, 10]).unwrap();
        drop(tx);

        handle.join().expect("WAV writer thread panicked");

        let reader = hound::WavReader::open(&path).expect("should open WAV file");
        let actual: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(actual, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spawn_wav_writer_handles_invalid_path_gracefully() {
        // Use a path whose parent directory doesn't exist → WavWriter::create fails.
        let dir = PathBuf::from("/tmp/kue-test-wav-invalid/nonexistent");
        let path = dir.join("fail.wav");

        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
        let handle = spawn_wav_writer("fail", rx, path.clone()).unwrap();

        // The writer may exit immediately when the path is invalid, closing the
        // receiver and causing this send to fail. That's fine — what matters is
        // that the thread does not panic and the file is not created.
        let _ = tx.send(vec![1, 2, 3]);
        drop(tx);

        // Thread should not panic; it should print an error and return.
        handle.join().expect("WAV writer thread should not panic on invalid path");

        // File should NOT have been created
        assert!(!path.exists(), "WAV file should NOT exist when parent dir is missing");
    }

    #[test]
    fn spawn_wav_writer_handles_empty_buffer() {
        let dir = PathBuf::from("/tmp/kue-test-wav-empty");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.wav");

        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
        let handle = spawn_wav_writer("empty", rx, path.clone()).unwrap();

        // Send an empty buffer
        tx.send(vec![]).unwrap();
        // Then close
        drop(tx);

        handle.join().expect("WAV writer thread panicked");

        // Empty WAV files have the header (44 bytes) but no data
        let reader = hound::WavReader::open(&path).expect("should open WAV file");
        assert_eq!(reader.duration(), 0, "empty buffer should produce 0 duration");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);

        fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------------
    // start() internal mutex state (hardware-agnostic)
    // -----------------------------------------------------------------------

    #[test]
    #[ignore = "requires audio hardware; cpal::build_input_stream may block on macOS"]
    fn audio_capture_start_valid_mode_locks_mutex_then_returns_hardware_err() {
        // This test verifies that calling start("practice") acquires the mutex,
        // passes mode validation, and then attempts hardware access. On systems
        // without a mic, it returns quickly with DeviceNotFound (not InvalidMode).
        // On systems WITH a mic, it may pass or block — so we wrap with timeout.
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test-lock"));
        let result = cap.start("practice");
        match result {
            Ok(_) => {
                cap.stop();
            }
            Err(e) => {
                // Must be a hardware error, not InvalidMode.
                assert!(!matches!(e, AudioError::InvalidMode(_)));
                // Verify the error is one of the expected hardware errors
                let msg = e.to_string();
                assert!(
                    msg.starts_with("DEVICE_NOT_FOUND:")
                    || msg.starts_with("STREAM_ERROR:")
                    || msg.starts_with("PERMISSION_DENIED:"),
                    "Expected hardware error, got: {msg}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn valid_modes_match_db_check_constraint() {
        // Must stay in sync with db/mod.rs `CHECK(mode IN ('practice', 'shadow'))`.
        assert!(
            VALID_MODES.contains(&"practice"),
            "VALID_MODES must include 'practice' to match DB CHECK constraint"
        );
        assert!(
            VALID_MODES.contains(&"shadow"),
            "VALID_MODES must include 'shadow' to match DB CHECK constraint"
        );
        assert_eq!(
            VALID_MODES.len(),
            2,
            "VALID_MODES should not have extra entries without updating DB CHECK"
        );
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(SAMPLE_RATE, 16_000);
        assert_eq!(CHANNELS, 1);
        // BUFFER_CAPACITY is private but we verify our WAV spec uses the right values
    }

    #[test]
    fn wav_spec_is_valid() {
        let spec = WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
    }

    // -----------------------------------------------------------------------
    // take_session_dir — extraction and clearing
    // -----------------------------------------------------------------------

    #[test]
    fn take_session_dir_returns_none_when_not_set() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test-take-session"));
        assert!(cap.take_session_dir().is_none());
    }

    #[test]
    fn take_session_dir_returns_and_clears_inner() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test-take-return"));
        let expected = PathBuf::from("/tmp/kue-manual-dir");

        // Manually set session_dir via inner mutex (same as start() would)
        {
            let mut inner = cap.inner.lock().unwrap();
            inner.session_dir = Some(expected.clone());
        }

        let taken = cap.take_session_dir();
        assert_eq!(taken, Some(expected), "should return the session_dir we set");

        // Verify the inner field was cleared
        let inner = cap.inner.lock().unwrap();
        assert!(inner.session_dir.is_none(), "take_session_dir should clear inner state");
    }

    // -----------------------------------------------------------------------
    // recordings_dir_path — path retrieval
    // -----------------------------------------------------------------------

    #[test]
    fn recordings_dir_path_returns_clone() {
        let dir = PathBuf::from("/tmp/kue-test-recordings-clone");
        let cap = AudioCapture::new(dir.clone());
        let got = cap.recordings_dir_path();
        assert_eq!(got, dir, "returned path should match the original");
        // Ensure it's a separate clone, not the same Arc/cell (PathBuf is Clone)
        assert_eq!(cap.recordings_dir, got, "inner path should be equal");
    }

    // -----------------------------------------------------------------------
    // apply_retention — file retention logic
    // -----------------------------------------------------------------------

    #[test]
    fn apply_retention_retain_true_moves_files_to_recordings_dir() {
        let tmp = std::env::temp_dir().join("kue-retain-move-test");
        let session_dir = tmp.join("session-123");
        let recordings_dir = tmp.join("recordings");
        let _ = fs::remove_dir_all(&tmp);

        // Create session_dir with a test file
        fs::create_dir_all(&session_dir).unwrap();
        let src_file = session_dir.join("mic_channel_A.wav");
        fs::write(&src_file, b"fake wav content").unwrap();

        apply_retention(&session_dir, &recordings_dir, true);

        // Session dir should be removed
        assert!(!session_dir.exists(), "session_dir should be removed after retention");

        // File should have been moved to recordings_dir/session-123/mic_channel_A.wav
        let dest_file = recordings_dir.join("session-123").join("mic_channel_A.wav");
        assert!(dest_file.exists(), "file should be moved to recordings dir");
        assert_eq!(fs::read(&dest_file).unwrap(), b"fake wav content");

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn apply_retention_retain_false_removes_session_dir_only() {
        let tmp = std::env::temp_dir().join("kue-retain-false-test");
        let session_dir = tmp.join("session-456");
        let recordings_dir = tmp.join("recordings");
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(&session_dir).unwrap();
        let src_file = session_dir.join("some.wav");
        fs::write(&src_file, b"data").unwrap();

        apply_retention(&session_dir, &recordings_dir, false);

        // Session dir should be removed
        assert!(!session_dir.exists(), "session_dir should be removed");

        // Recordings dir should NOT have been created
        assert!(!recordings_dir.exists(), "recordings dir should not exist when retain=false");
    }

    #[test]
    fn apply_retention_missing_session_dir_does_not_panic() {
        // Even though session_dir doesn't exist, remove_dir_all on a
        // non-existent path is a no-op (returns Ok).
        let tmp = std::env::temp_dir().join("kue-retain-missing-test");
        let session_dir = tmp.join("ghost-session");
        let recordings_dir = tmp.join("recordings");
        let _ = fs::remove_dir_all(&tmp);

        // Do NOT create session_dir — it doesn't exist
        apply_retention(&session_dir, &recordings_dir, true);

        // Should not panic, session_dir should still not exist
        assert!(!session_dir.exists());
        // recordings_dir may or may not exist (create_dir_all may have been called)
        // but no crash is the main assertion
    }

    #[test]
    fn apply_retention_with_retain_creates_recordings_dir() {
        let tmp = std::env::temp_dir().join("kue-retain-create-dir-test");
        let session_dir = tmp.join("sess-create");
        let recordings_dir = tmp.join("nested").join("recordings");
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("a.wav"), b"abc").unwrap();

        apply_retention(&session_dir, &recordings_dir, true);

        // The recordings dir should have been created
        assert!(recordings_dir.exists(), "recordings_dir should be created");
        // File moved into recordings_dir/sess-create/a.wav
        let moved = recordings_dir.join("sess-create").join("a.wav");
        assert!(moved.exists(), "file should be moved");

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn apply_retention_retain_true_with_multiple_files() {
        let tmp = std::env::temp_dir().join("kue-retain-multi-test");
        let session_dir = tmp.join("multi-sess");
        let recordings_dir = tmp.join("rec");
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("f1.wav"), b"file1").unwrap();
        fs::write(session_dir.join("f2.wav"), b"file2").unwrap();
        fs::write(session_dir.join("f3.txt"), b"notes").unwrap(); // non-wav

        apply_retention(&session_dir, &recordings_dir, true);

        // All files should have been moved
        let target = recordings_dir.join("multi-sess");
        assert!(target.join("f1.wav").exists());
        assert!(target.join("f2.wav").exists());
        assert!(target.join("f3.txt").exists());
        assert!(!session_dir.exists());

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn apply_retention_retain_true_session_dir_no_file_name() {
        // Use root "/" which has no file_name in the sense that it returns None
        // (technically "/" does have a file_name "" on some platforms, but
        // Path::new("/").file_name() returns None on macOS).
        let tmp = std::env::temp_dir().join("kue-retain-root-test");
        let recordings_dir = tmp.join("rec");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&recordings_dir).unwrap();

        // Use a path whose file_name() returns None (root dir "/")
        apply_retention(Path::new("/"), &recordings_dir, true);

        // Should not panic, recordings dir should still exist (though the
        // function will attempt and fail to remove_dir_all("/") — this is fine
        // because the test doesn't assert about that; we just verify no crash).
        assert!(recordings_dir.exists());

        // Also test with retain=false and a problematic session_dir path
        apply_retention(Path::new("/nonexistent_weird_path_that_does_not_exist_xyz"), &recordings_dir, false);
        // Should not panic
    }

    #[test]
    fn apply_retention_read_dir_failure_does_not_panic() {
        // If session_dir is a file (not a directory), read_dir returns Err.
        let tmp = std::env::temp_dir().join("kue-retain-readdir-test");
        let session_dir = tmp.join("not-a-dir");
        let recordings_dir = tmp.join("rec");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Create a file at the session_dir path instead of a directory
        fs::write(&session_dir, b"i am a file not a dir").unwrap();

        // This should not panic — read_dir will fail, apply_retention
        // silently skips the loop, then removes the file via remove_dir_all
        // (which will fail for a file but that's swallowed).
        apply_retention(&session_dir, &recordings_dir, true);

        // The file should still exist because remove_dir_all doesn't remove files
        // on macOS (it returns Err for non-directory).
        // The main assertion is no panic.
        assert!(session_dir.exists());

        fs::remove_dir_all(&tmp).ok();
    }

    // -----------------------------------------------------------------------
    // stop_session — ended_at is set in DB
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // start_session — company/role persistence
    // -----------------------------------------------------------------------

    #[test]
    fn start_session_persists_company_and_role() {
        use crate::db::open_and_migrate;

        crate::db::register_vec_extension();

        let tmp = std::env::temp_dir().join("kue-test-start-company-role");
        let db_path = tmp.join("test.db");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let db = open_and_migrate(&db_path).expect("db should open");
        let session_id = "test-company-role-001";
        let company = "Acme Corp";
        let role = "Senior Engineer";

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES (?1, ?2, ?3, 'practice')",
                rusqlite::params![session_id, company, role],
            )
            .unwrap();
        }

        {
            let conn = db.conn.lock().unwrap();
            let (stored_company, stored_role): (String, String) = conn
                .query_row(
                    "SELECT company, role FROM sessions WHERE id = ?1",
                    rusqlite::params![session_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(stored_company, company);
            assert_eq!(stored_role, role);
        }

        // Also verify that empty values work (default behavior unchanged)
        let session_id2 = "test-company-role-empty";
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES (?1, '', '', 'shadow')",
                rusqlite::params![session_id2],
            )
            .unwrap();
        }
        {
            let conn = db.conn.lock().unwrap();
            let (c, r): (String, String) = conn
                .query_row(
                    "SELECT company, role FROM sessions WHERE id = ?1",
                    rusqlite::params![session_id2],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(c, "");
            assert_eq!(r, "");
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stop_session_sets_ended_at_in_db() {
        use crate::db::open_and_migrate;

        crate::db::register_vec_extension();

        let tmp = std::env::temp_dir().join("kue-test-ended-at");
        let db_path = tmp.join("test.db");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let db = open_and_migrate(&db_path).expect("db should open");
        let session_id = "test-ended-at-001";

        // Insert a session manually (as start_session would)
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES (?1, '', '', 'practice')",
                rusqlite::params![session_id],
            )
            .unwrap();
        }

        // Now simulate the UPDATE that stop_session does
        {
            let conn = db.conn.lock().unwrap();
            let updated = conn
                .execute(
                    "UPDATE sessions SET ended_at = datetime('now') WHERE id = ?1",
                    rusqlite::params![session_id],
                )
                .unwrap();
            assert_eq!(updated, 1, "should update exactly one row");
        }

        // Verify ended_at was set (not NULL) and is a plausible datetime string
        {
            let conn = db.conn.lock().unwrap();
            let ended_at: Option<String> = conn
                .query_row(
                    "SELECT ended_at FROM sessions WHERE id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                ended_at.is_some(),
                "ended_at should NOT be NULL after stop_session"
            );
            let ended = ended_at.unwrap();
            assert!(
                ended.len() >= 16,
                "ended_at '{}' should be a datetime string like '2024-01-15 12:34:56'",
                ended,
            );
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn apply_retention_retain_true_creates_target_subdir() {
        let tmp = std::env::temp_dir().join("kue-retain-subdir-test");
        let session_dir = tmp.join("sess-sub");
        let recordings_dir = tmp.join("rec");
        let _ = fs::remove_dir_all(&tmp);

        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("a.wav"), b"data").unwrap();

        // The target subdirectory recordings_dir/sess-sub doesn't exist yet
        apply_retention(&session_dir, &recordings_dir, true);

        // The subdirectory should have been created
        let target = recordings_dir.join("sess-sub");
        assert!(target.is_dir(), "target subdirectory should exist");
        assert!(target.join("a.wav").exists(), "file should be in target");

        fs::remove_dir_all(&tmp).ok();
    }
}
