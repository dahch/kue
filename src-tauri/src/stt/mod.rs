pub mod batch;
pub mod cli;
pub mod ffi;
mod pipeline;
pub mod provisioning;
mod vad;

pub use pipeline::STTPipeline;
pub use vad::SimpleVAD;

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::db::Database;
use crate::types::Speaker;

static MOONSHINE_BASE: OnceLock<PathBuf> = OnceLock::new();

/// Returns the managed lib directory set by the provisioning module,
/// if available.
pub(crate) fn managed_lib_dir() -> Option<PathBuf> {
    MOONSHINE_BASE.get().map(|b| b.join("lib"))
}

/// Root-mean-square amplitude of i16 audio samples.
pub(crate) fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Insert a single transcript line into the database.
pub(crate) fn persist_transcript_line(
    db: &Database,
    session_id: &str,
    text: &str,
    speaker: &Speaker,
    started_at_ms: u64,
    ended_at_ms: u64,
) {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to lock DB: {e}");
            return;
        }
    };

    if let Err(e) = conn.execute(
        "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            session_id,
            speaker.as_db_str(),
            text,
            started_at_ms,
            ended_at_ms
        ],
    ) {
        log::error!("Failed to persist transcript line: {e}");
    }
}

/// Create an STT engine: FFI if available, otherwise CLI fallback.
pub fn create_engine(config: &STTConfig) -> Box<dyn STTEngine> {
    if ffi::MoonshineFFIEngine::is_available() {
        log::info!("Using Moonshine FFI engine");
        Box::new(ffi::MoonshineFFIEngine::new())
    } else if config.use_cli_fallback {
        log::warn!("Moonshine lib not found, falling back to CLI engine");
        Box::new(cli::MoonshineCLIEngine::new())
    } else {
        log::warn!("No Moonshine lib and CLI fallback disabled");
        Box::new(cli::MoonshineCLIEngine::new())
    }
}

pub trait STTEngine: Send {
    fn load(&mut self, model_path: &PathBuf, language: &str) -> Result<(), String>;
    fn transcribe_audio_chunk(&self, chunk: &[i16]) -> Option<String>;
}

impl<T: STTEngine + ?Sized> STTEngine for Box<T> {
    fn load(&mut self, model_path: &PathBuf, language: &str) -> Result<(), String> {
        (**self).load(model_path, language)
    }
    fn transcribe_audio_chunk(&self, chunk: &[i16]) -> Option<String> {
        (**self).transcribe_audio_chunk(chunk)
    }
}

#[derive(Debug, Clone)]
pub struct STTConfig {
    pub model_path: PathBuf,
    pub language: String,
    pub vad_threshold: f32,
    pub min_speech_duration_ms: u64,
    pub silence_timeout_ms: u64,
    pub sample_rate: u32,
    pub use_cli_fallback: bool,
    /// Hard cap on segment duration (ms) to avoid unbounded buffers
    /// when the speaker talks continuously without pauses. The pipeline
    /// will flush the current segment at this boundary even if VAD has
    /// not detected silence.
    pub max_segment_duration_ms: u64,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            model_path: Self::default_model_path(),
            language: "en".to_string(),
            // Very low threshold because the audio is already in i16 scale;
            // combined with the loopback gain this catches quiet speech.
            vad_threshold: 0.015,
            // 200ms minimum avoids false starts from tiny noises.
            min_speech_duration_ms: 200,
            // 600ms silence gap before ending a segment — enough for natural
            // pauses inside a sentence without merging separate utterances.
            silence_timeout_ms: 600,
            sample_rate: 16_000,
            use_cli_fallback: true,
            // Force-flush after 12s of continuous speech even without silence;
            // prevents the engine from receiving a buffer that is too large
            // for the streaming model and blocks the pipeline indefinitely.
            max_segment_duration_ms: 12_000,
        }
    }
}

impl STTConfig {
    /// Discover the Moonshine model directory at well-known locations,
    /// preferring the managed app-data path, then dev paths, falling
    /// back to a relative path if none is found.
    pub fn default_model_path() -> PathBuf {
        // Managed path (set by provisioning module after first-launch download).
        if let Some(base) = MOONSHINE_BASE.get() {
            let p = base.join("models").join("en").join("medium-streaming");
            if p.exists() {
                return p;
            }
        }

        // Dev paths (manual install from Sprint 1).
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(&home);
            let p = home_path
                .join(".local")
                .join("share")
                .join("moonshine")
                .join("models")
                .join("en")
                .join("medium-streaming");
            if p.exists() {
                return p;
            }
            let p = home_path
                .join("moonshine-models")
                .join("en")
                .join("medium-streaming");
            if p.exists() {
                return p;
            }
        }
        PathBuf::from("moonshine-models/en/medium-streaming")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // STTConfig defaults
    // -----------------------------------------------------------------------

    #[test]
    fn stt_config_default_model_path() {
        let config = STTConfig::default();
        assert_eq!(
            config.model_path,
            PathBuf::from("moonshine-models/en/medium-streaming")
        );
    }

    #[test]
    fn stt_config_default_language() {
        let config = STTConfig::default();
        assert_eq!(config.language, "en");
    }

    #[test]
    fn stt_config_default_vad_threshold() {
        let config = STTConfig::default();
        assert!((config.vad_threshold - 0.015).abs() < f32::EPSILON);
    }

    #[test]
    fn stt_config_default_min_speech_duration() {
        let config = STTConfig::default();
        assert_eq!(config.min_speech_duration_ms, 200);
    }

    #[test]
    fn stt_config_default_silence_timeout() {
        let config = STTConfig::default();
        assert_eq!(config.silence_timeout_ms, 600);
    }

    #[test]
    fn stt_config_default_sample_rate() {
        let config = STTConfig::default();
        assert_eq!(config.sample_rate, 16_000);
    }

    #[test]
    fn stt_config_default_use_cli_fallback() {
        let config = STTConfig::default();
        assert!(config.use_cli_fallback);
    }

    #[test]
    fn stt_config_default_max_segment_duration() {
        let config = STTConfig::default();
        assert_eq!(config.max_segment_duration_ms, 12_000);
    }

    // -----------------------------------------------------------------------
    // STTEngine delegation through Box<T>
    // -----------------------------------------------------------------------

    /// A minimal engine that records whether load() was called.
    struct TrackingEngine {
        load_called: bool,
    }

    impl TrackingEngine {
        fn new() -> Self {
            Self { load_called: false }
        }
    }

    impl STTEngine for TrackingEngine {
        fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
            self.load_called = true;
            Ok(())
        }

        fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
            Some("hello".to_string())
        }
    }

    #[test]
    fn box_stt_engine_forwards_load() {
        let inner = TrackingEngine::new();
        let mut engine: Box<dyn STTEngine> = Box::new(inner);
        let result = engine.load(&PathBuf::from("model"), "en");
        assert!(result.is_ok(), "Box<dyn STTEngine>::load should delegate");
    }

    #[test]
    fn box_stt_engine_forwards_transcribe() {
        let inner = TrackingEngine::new();
        let engine: Box<dyn STTEngine> = Box::new(inner);
        let result = engine.transcribe_audio_chunk(&[1, 2, 3]);
        assert_eq!(result.as_deref(), Some("hello"));
    }

    #[test]
    fn box_stt_engine_load_propagates_error() {
        struct FailingEngine;

        impl STTEngine for FailingEngine {
            fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
                Err("load_failed".to_string())
            }
            fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
                None
            }
        }

        let mut engine: Box<dyn STTEngine> = Box::new(FailingEngine);
        let result = engine.load(&PathBuf::from("x"), "en");
        assert_eq!(result.unwrap_err(), "load_failed");
    }

    #[test]
    fn box_stt_engine_transcribe_returns_none() {
        struct SilentEngine;

        impl STTEngine for SilentEngine {
            fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
                Ok(())
            }
            fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
                None
            }
        }

        let engine: Box<dyn STTEngine> = Box::new(SilentEngine);
        assert!(engine.transcribe_audio_chunk(&[0]).is_none());
    }

    // -----------------------------------------------------------------------
    // Send + STTEngine bounds
    // -----------------------------------------------------------------------

    #[test]
    fn box_stt_engine_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Box<dyn STTEngine>>();
    }

    // -----------------------------------------------------------------------
    // rms — additional edge cases not covered in vad.rs or cli.rs
    // -----------------------------------------------------------------------

    #[test]
    fn rms_single_positive_value() {
        let r = rms(&[100]);
        assert!((r - 100.0).abs() < 1e-6);
    }

    #[test]
    fn rms_single_negative_value() {
        // RMS squares the value so sign doesn't matter
        let r = rms(&[-100]);
        assert!((r - 100.0).abs() < 1e-6);
    }

    #[test]
    fn rms_alternating_values() {
        let r = rms(&[100, -100, 100, -100]);
        assert!((r - 100.0).abs() < 1e-6);
    }

    #[test]
    fn rms_large_dataset() {
        let samples: Vec<i16> = vec![1000; 100_000];
        let r = rms(&samples);
        assert!((r - 1000.0).abs() < 1.0);
    }

    #[test]
    fn rms_mixed_magnitudes() {
        // RMS of [0, 300] should be ~212.13
        let r = rms(&[0, 300]);
        assert!((r - 212.13).abs() < 0.01);
    }

    #[test]
    fn rms_all_max_values() {
        let samples = vec![i16::MAX; 1000];
        let r = rms(&samples);
        assert!((r - 32767.0).abs() < 1.0);
    }

    #[test]
    fn rms_all_min_values() {
        let samples = vec![i16::MIN; 1000];
        let r = rms(&samples);
        assert!((r - 32768.0).abs() < 1.0);
    }

    // -----------------------------------------------------------------------
    // create_engine branch coverage
    // -----------------------------------------------------------------------

    /// Returns true if the FFI engine would be available (shared lib found).
    /// This is the same check `create_engine` uses internally.
    #[test]
    fn create_engine_fallback_when_ffi_unavailable() {
        // In test environments, no Moonshine shared library should be available,
        // so the engine will fall back to CLI fallback path.
        let config = STTConfig::default();
        let engine = create_engine(&config);
        // The concrete type is erased behind Box<dyn STTEngine>, but we can
        // verify it produces a working engine by calling transcribe.
        let result = engine.transcribe_audio_chunk(&[]);
        // CLI engine returns None for empty chunks (rms < 0.005)
        assert!(result.is_none());
    }

    #[test]
    fn create_engine_uses_cli_when_ffi_unavailable_and_fallback_disabled() {
        // Even when use_cli_fallback=false, the engine should still create
        // something (there's no alternative engine type).
        let mut config = STTConfig::default();
        config.use_cli_fallback = false;
        let engine = create_engine(&config);
        let result = engine.transcribe_audio_chunk(&[]);
        assert!(result.is_none());
    }
}
