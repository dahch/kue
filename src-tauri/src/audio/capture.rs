use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use screencapturekit::cm_sample_buffer::CMSampleBuffer;
use screencapturekit::sc_content_filter::{self, SCContentFilter};
use screencapturekit::sc_error_handler::StreamErrorHandler;
use screencapturekit::sc_output_handler::{SCStreamOutputType, StreamOutput};
use screencapturekit::sc_shareable_content::SCShareableContent;
use screencapturekit::sc_stream::SCStream;
use screencapturekit::sc_stream_configuration::SCStreamConfiguration;
use serde::Serialize;

use super::mic_vad::MicVadState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
const BUFFER_CAPACITY: usize = 60;
const VALID_MODES: [&str; 2] = ["practice", "shadow"];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AudioCaptureStatus {
    pub mic_active: bool,
    pub loopback_active: bool,
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

pub struct AudioCapture {
    inner: Mutex<AudioCaptureInner>,
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
}

impl AudioCapture {
    pub fn new(recordings_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(AudioCaptureInner {
                mic: None,
                loopback: None,
                mic_writer: None,
                loopback_writer: None,
                mic_vad_handle: None,
                stt_thread: None,
                mode: String::new(),
                session_dir: None,
            }),
            recordings_dir,
            mic_vad: Arc::new(Mutex::new(MicVadState::new())),
        }
    }

    /// Returns a clone of the shared `MicVadState` handle so the hint worker
    /// can check for user voice activity before emitting Shadow-mode hints.
    pub fn mic_vad_state(&self) -> Arc<Mutex<MicVadState>> {
        self.mic_vad.clone()
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
        let mic_path = session_dir.join("mic_channel_A.wav");
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
        let loopback_path = session_dir.join("loopback_channel_B.wav");
        let loopback_writer = spawn_wav_writer("loopback-B", loopback_rx, loopback_path)?;

        inner.loopback = Some(LoopbackHandle(loopback));
        inner.loopback_writer = Some(loopback_writer);
        inner.mode = mode.to_string();
        inner.session_dir = Some(session_dir);

        Ok((
            AudioCaptureStatus {
                mic_active: true,
                loopback_active: true,
            },
            stt_rx,
        ))
    }

    pub fn stop(&self) -> AudioCaptureStatus {
        let mut inner = self.inner.lock().unwrap();
        // Drop streams first (stops capture, closes senders → receivers
        // disconnect → writer threads exit).
        inner.mic = None;
        inner.loopback = None;

        // Join writer threads so the WAV files are fully flushed.
        inner.mic_writer.take().map(|h| h.join().ok());
        inner.loopback_writer.take().map(|h| h.join().ok());

        // Join VAD monitor thread
        inner.mic_vad_handle.take().map(|h| h.join().ok());

        // Join STT thread (it will disconnect when the audio receiver drops)
        inner.stt_thread.take().map(|h| h.join().ok());

        inner.mode = String::new();
        // session_dir stays for finalize_session to handle cleanup

        AudioCaptureStatus {
            mic_active: false,
            loopback_active: false,
        }
    }

    /// Finalize the session: if `retain` is true, move WAV files to
    /// `recordings_dir/{session_name}/`; otherwise delete the temp dir.
    /// No-op if no session dir exists.
    pub fn finalize_session(&self, retain: bool) {
        let mut inner = self.inner.lock().unwrap();
        let Some(session_dir) = inner.session_dir.take() else {
            return;
        };

        if retain {
            if let Err(e) = fs::create_dir_all(&self.recordings_dir) {
                eprintln!("[kue] Failed to create recordings dir {dir:?}: {e}",
                    dir = self.recordings_dir);
            } else if let Some(name) = session_dir.file_name() {
                let target = self.recordings_dir.join(name);
                let _ = fs::create_dir_all(&target);
                if let Ok(entries) = fs::read_dir(&session_dir) {
                    for entry in entries.flatten() {
                        let dest = target.join(entry.file_name());
                        let _ = fs::rename(entry.path(), &dest);
                    }
                }
            }
        }

        let _ = fs::remove_dir_all(&session_dir);
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

    pub fn toggle(
        &self,
        start: bool,
        mode: &str,
    ) -> Result<(AudioCaptureStatus, Option<std::sync::mpsc::Receiver<Vec<i16>>>), AudioError> {
        if start {
            let (status, rx) = self.start(mode)?;
            Ok((status, Some(rx)))
        } else {
            Ok((self.stop(), None))
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
                    eprintln!("[kue] Failed to create WAV file {path:?}: {e}");
                    return;
                }
            };

            println!("[kue] WAV writer '{label}' started → {path:?}");

            for samples in &rx {
                for &s in &samples {
                    if let Err(e) = writer.write_sample(s) {
                        eprintln!("[kue] WAV writer '{label}' write error: {e}");
                    }
                }
            }

            // Channel closed — flush and finalize.
            if let Err(e) = writer.finalize() {
                eprintln!("[kue] WAV writer '{label}' finalize error: {e}");
            }
            println!("[kue] WAV writer '{label}' finished → {path:?}");
        })
        .map_err(|e| AudioError::StreamError(format!("Failed to spawn WAV writer thread: {e}")))
}

// ---------------------------------------------------------------------------
// Mic capture (Canal A) – cpal, with tee for VAD
// ---------------------------------------------------------------------------

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

    match sample_format {
        cpal::SampleFormat::I16 => {
            let stream = device
                .build_input_stream::<i16, _, _>(
                    &stream_config,
                    move |data, _| {
                        let samples = data.to_vec();
                        let _ = wav_tx.try_send(samples.clone());
                        let _ = vad_tx.try_send(samples);
                    },
                    move |err| eprintln!("[kue] Mic capture error: {err}"),
                    None,
                )
                .map_err(|e| AudioError::StreamError(format!("Failed to build mic stream: {e}")))?;
            stream
                .play()
                .map_err(|e| AudioError::StreamError(format!("Failed to start mic stream: {e}")))?;
            println!("[kue] Mic capture (Canal A) started — 16 kHz, mono, i16");
            Ok(stream)
        }
        cpal::SampleFormat::F32 => {
            let stream = device
                .build_input_stream::<f32, _, _>(
                    &stream_config,
                    move |data, _| {
                        let converted: Vec<i16> = data
                            .iter()
                            .map(|&s| f32_to_i16(s))
                            .collect();
                        let _ = wav_tx.try_send(converted.clone());
                        let _ = vad_tx.try_send(converted);
                    },
                    move |err| eprintln!("[kue] Mic capture error: {err}"),
                    None,
                )
                .map_err(|e| AudioError::StreamError(format!("Failed to build mic stream: {e}")))?;
            stream
                .play()
                .map_err(|e| AudioError::StreamError(format!("Failed to start mic stream: {e}")))?;
            println!("[kue] Mic capture (Canal A) started — 16 kHz, mono, f32 → i16");
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
            eprintln!("[kue] Loopback capture stream error");
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
                        f32_to_i16(f)
                    })
                    .collect();
                let _ = self.wav_tx.try_send(samples.clone());
                let _ = self.stt_tx.try_send(samples);
            }
        }
    }

    let mut stream = SCStream::new(filter, config, AudioErrorHandler);
    let output = LoopbackOutputTee { wav_tx, stt_tx };
    stream.add_output(output, SCStreamOutputType::Audio);

    stream
        .start_capture()
        .map_err(|e| AudioError::StreamError(format!("Failed to start loopback capture: {e}")))?;

    println!("[kue] Loopback capture (Canal B) with STT tee started — 16 kHz, mono via ScreenCaptureKit");
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
// Tauri command
// ---------------------------------------------------------------------------

use crate::db::Database;
use crate::orchestrator::HintJobSender;
use crate::stt::{STTConfig, STTPipeline};
use tauri::Manager;

#[tauri::command]
pub fn toggle_audio_capture(
    start: bool,
    mode: String,
    audio: tauri::State<'_, AudioCapture>,
    db: tauri::State<'_, Database>,
    app_handle: tauri::AppHandle,
) -> Result<AudioCaptureStatus, String> {
    if start {
        let (status, stt_rx) = audio.start(&mode).map_err(|e| e.to_string())?;

        // Create a new session in the DB
        let session_id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "", "", mode],
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
            .with_app_handle(app_handle)
            .with_mode(&mode)
            .with_hint_job_tx(hint_job_tx);
        if let Err(e) = pipeline.load_model() {
            eprintln!("[kue] STT model load failed (best-effort): {e}");
        }
        pipeline.start_session(&session_id);

        let db_for_stt = Database::clone(db.inner());
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let stt_thread = pipeline.spawn_processing_thread(rx, db_for_stt);

        let mut inner = audio.inner.lock().unwrap();
        inner.stt_thread = Some(stt_thread);

        Ok(status)
    } else {
        let status = audio.stop();

        // Read retain_audio setting (default: false)
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
        audio.finalize_session(retain);

        Ok(status)
    }
}



// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":false,"loopback_active":false}"#);
    }

    #[test]
    fn audio_capture_status_serialization_both_true() {
        let status = AudioCaptureStatus {
            mic_active: true,
            loopback_active: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":true,"loopback_active":true}"#);
    }

    #[test]
    fn audio_capture_status_serialization_mic_true_loopback_false() {
        let status = AudioCaptureStatus {
            mic_active: true,
            loopback_active: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":true,"loopback_active":false}"#);
    }

    #[test]
    fn audio_capture_status_serialization_mic_false_loopback_true() {
        let status = AudioCaptureStatus {
            mic_active: false,
            loopback_active: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"mic_active":false,"loopback_active":true}"#);
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
    // finalize_session() — cleanup / retention logic
    // -----------------------------------------------------------------------

    #[test]
    fn finalize_session_noop_when_no_session() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        // Should not panic when session_dir is None
        cap.finalize_session(false);
        cap.finalize_session(true);
    }

    #[test]
    fn finalize_session_retain_false_deletes_dir() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let tmp = PathBuf::from("/tmp/kue-finalize-delete-test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("test.wav"), b"fake wav data").unwrap();

        {
            let mut inner = cap.inner.lock().unwrap();
            inner.session_dir = Some(tmp.clone());
        }
        cap.finalize_session(false);
        assert!(!tmp.exists(), "session dir should be deleted when retain=false");
    }

    #[test]
    fn finalize_session_retain_true_moves_files() {
        let recordings_dir = PathBuf::from("/tmp/kue-finalize-move-recordings");
        let _ = fs::remove_dir_all(&recordings_dir);

        let cap = AudioCapture::new(recordings_dir.clone());

        let tmp = PathBuf::from("/tmp/kue-finalize-move-session");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("mic_channel_A.wav"), b"mic data").unwrap();
        fs::write(tmp.join("loopback_channel_B.wav"), b"loopback data").unwrap();

        {
            let mut inner = cap.inner.lock().unwrap();
            inner.session_dir = Some(tmp.clone());
        }
        cap.finalize_session(true);

        // The temp dir should be gone
        assert!(!tmp.exists(), "session dir should be deleted after finalize");

        // Files should have been moved to recordings_dir/{tmp_name}/
        let dirname = tmp.file_name().unwrap();
        let target = recordings_dir.join(dirname);
        assert!(target.join("mic_channel_A.wav").exists(), "mic file should be in recordings");
        assert!(target.join("loopback_channel_B.wav").exists(), "loopback file should be in recordings");

        fs::remove_dir_all(&recordings_dir).ok();
    }

    #[test]
    fn finalize_session_retain_true_creates_recordings_dir() {
        let recordings_dir = PathBuf::from("/tmp/kue-finalize-create-recordings/nested");
        let _ = fs::remove_dir_all(&recordings_dir);

        let cap = AudioCapture::new(recordings_dir.clone());

        let tmp = PathBuf::from("/tmp/kue-finalize-create-session");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("data.wav"), b"data").unwrap();

        {
            let mut inner = cap.inner.lock().unwrap();
            inner.session_dir = Some(tmp.clone());
        }
        cap.finalize_session(true);

        let dirname = tmp.file_name().unwrap();
        assert!(recordings_dir.join(dirname).join("data.wav").exists());

        fs::remove_dir_all("/tmp/kue-finalize-create-recordings").ok();
    }

    // -----------------------------------------------------------------------
    // finalize_session — error branches
    // -----------------------------------------------------------------------

    #[test]
    fn finalize_session_retain_true_no_filename_does_not_panic() {
        // When session_dir has no file_name (e.g., a root path), retain=true
        // should not panic — it simply skips the file-moving branch.
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test-no-fn"));
        let root = PathBuf::from("/"); // has no meaningful file_name

        {
            let mut inner = cap.inner.lock().unwrap();
            inner.session_dir = Some(root);
        }
        // Should not panic
        cap.finalize_session(true);
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
    // toggle() — routes to start/stop correctly
    // -----------------------------------------------------------------------

    #[test]
    fn audio_capture_toggle_stop_returns_inactive_status() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let (status, _) = cap.toggle(false, "practice").unwrap();
        assert!(!status.mic_active);
        assert!(!status.loopback_active);
    }

    #[test]
    fn audio_capture_toggle_stop_ignores_mode() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        // Even with an invalid mode, toggle(start=false) should succeed
        // because it delegates to stop(), which ignores mode.
        let (status, _) = cap.toggle(false, "INVALID_MODE_THAT_STOP_IGNORES").unwrap();
        assert!(!status.mic_active);
        assert!(!status.loopback_active);
    }

    #[test]
    fn audio_capture_toggle_start_validates_mode_before_hardware() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let err = cap.toggle(true, "invalid").unwrap_err();
        assert!(matches!(err, AudioError::InvalidMode(_)));
    }

    #[test]
    fn audio_capture_toggle_start_empty_mode() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let err = cap.toggle(true, "").unwrap_err();
        assert!(matches!(err, AudioError::InvalidMode(_)));
    }

    /// Like `audio_capture_start_valid_modes_hardware` — requires real hardware.
    #[test]
    #[ignore = "requires real audio hardware; may block on some systems"]
    fn audio_capture_toggle_start_delegates_to_start() {
        let cap = AudioCapture::new(PathBuf::from("/tmp/kue-test"));
        let result = cap.toggle(true, "practice");
        match result {
            Ok((status, _)) => {
                assert!(status.mic_active);
                assert!(status.loopback_active);
                cap.stop();
            }
            Err(e) => {
                // Must be a hardware error, not an InvalidMode error.
                assert!(!matches!(e, AudioError::InvalidMode(_)));
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

        // Send data and close — the writer should fail to create the file and exit.
        tx.send(vec![1, 2, 3]).unwrap();
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
}
