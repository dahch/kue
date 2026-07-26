pub mod cli;
pub mod ffi;
mod pipeline;
mod vad;

pub use pipeline::STTPipeline;
pub use vad::SimpleVAD;

use std::path::PathBuf;

/// Root-mean-square amplitude of i16 audio samples.
pub(crate) fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
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
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            model_path: Self::default_model_path(),
            language: "en".to_string(),
            vad_threshold: 0.02,
            min_speech_duration_ms: 200,
            silence_timeout_ms: 600,
            sample_rate: 16_000,
            use_cli_fallback: true,
        }
    }
}

impl STTConfig {
    /// Discover the Moonshine model directory at well-known locations,
    /// falling back to a relative path if none is found.
    pub fn default_model_path() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(&home);
            let p = home_path.join(".local").join("share").join("moonshine").join("models").join("en").join("medium-streaming");
            if p.exists() {
                return p;
            }
            let p = home_path.join("moonshine-models").join("en").join("medium-streaming");
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
        assert!((config.vad_threshold - 0.02).abs() < f32::EPSILON);
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
}
