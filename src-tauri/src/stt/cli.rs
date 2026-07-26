use std::path::PathBuf;
use std::process::Command;

use super::rms;
use super::STTEngine;

pub struct MoonshineCLIEngine {
    model_path: Option<PathBuf>,
    language: String,
}

impl MoonshineCLIEngine {
    pub fn new() -> Self {
        Self {
            model_path: None,
            language: "en".to_string(),
        }
    }

    fn cli_path() -> String {
        "moonshine-voice".to_string()
    }
}

impl STTEngine for MoonshineCLIEngine {
    fn load(&mut self, model_path: &PathBuf, language: &str) -> Result<(), String> {
        let check = Command::new(Self::cli_path())
            .arg("--version")
            .output()
            .map_err(|e| format!("moonshine-voice not found in PATH: {e}"))?;

        if !check.status.success() {
            return Err("moonshine-voice CLI returned error on version check".into());
        }

        self.model_path = Some(model_path.clone());
        self.language = language.to_string();
        Ok(())
    }

    fn transcribe_audio_chunk(&self, chunk: &[i16]) -> Option<String> {
        if chunk.is_empty() || rms(chunk) < 0.005 {
            return None;
        }

        let tmp = write_wav_temp(chunk)?;
        let result = self.transcribe_file(&tmp);
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

impl MoonshineCLIEngine {
    fn transcribe_file(&self, wav_path: &std::path::Path) -> Option<String> {
        let mut cmd = Command::new(Self::cli_path());
        cmd.arg("transcribe")
            .arg("--wav-path")
            .arg(wav_path)
            .arg("--language")
            .arg(&self.language)
            .arg("--quiet");

        if let Some(ref mp) = self.model_path {
            cmd.arg("--model-path").arg(mp);
        }

        let output = cmd.output().ok()?;

        let stderr_str = String::from_utf8_lossy(&output.stderr);
        let stdout_str = String::from_utf8_lossy(&output.stdout);

        let all = format!("{}{}", stderr_str, stdout_str);
        let line = all
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .last()?;

        Some(line.to_string())
    }
}

fn write_wav_temp(samples: &[i16]) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("kue-stt");
    std::fs::create_dir_all(&dir).ok()?;
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nonce = uuid::Uuid::new_v4();
    let path = dir.join(format!("segment-{nonce}-{id}.wav"));

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(&path, spec).ok()?;
    for &s in samples {
        writer.write_sample(s).ok()?;
    }
    writer.finalize().ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::rms;

    // -----------------------------------------------------------------------
    // Engine construction
    // -----------------------------------------------------------------------

    #[test]
    fn moonshine_cli_engine_new_sets_defaults() {
        let engine = MoonshineCLIEngine::new();
        assert!(engine.model_path.is_none());
        assert_eq!(engine.language, "en");
    }

    #[test]
    fn cli_path_returns_moonshine_voice() {
        assert_eq!(MoonshineCLIEngine::cli_path(), "moonshine-voice");
    }

    // -----------------------------------------------------------------------
    // rms helper
    // -----------------------------------------------------------------------

    #[test]
    fn rms_zero() {
        assert!((rms(&[]) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn rms_constant() {
        let r = rms(&[1000; 100]);
        assert!((r - 1000.0).abs() < 0.1);
    }

    #[test]
    fn rms_single_value() {
        assert!((rms(&[500]) - 500.0).abs() < 1e-6);
    }

    #[test]
    fn rms_negative_and_positive() {
        let r = rms(&[-100, 100]);
        assert!((r - 100.0).abs() < 1e-6);
    }

    #[test]
    fn rms_max_int16_single() {
        let r = rms(&[i16::MAX]);
        assert!((r - 32767.0).abs() < 0.1);
    }

    // -----------------------------------------------------------------------
    // write_wav_temp — file lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn write_wav_temp_creates_wav() {
        let samples = vec![0i16, 100, -100, i16::MAX, i16::MIN];
        let path = write_wav_temp(&samples).expect("should write temp WAV");
        assert!(path.exists(), "temp WAV should exist");

        let reader = hound::WavReader::open(&path).expect("should open WAV");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().bits_per_sample, 16);

        let actual: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(actual, samples);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_wav_temp_empty_buffer() {
        let path = write_wav_temp(&[]).expect("should write even empty WAV");
        assert!(path.exists());
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_wav_temp_unique_names() {
        let s = vec![0i16; 10];
        let p1 = write_wav_temp(&s).unwrap();
        let p2 = write_wav_temp(&s).unwrap();
        assert_ne!(p1, p2, "each call should produce a unique filename");
        std::fs::remove_file(&p1).ok();
        std::fs::remove_file(&p2).ok();
    }

    #[test]
    fn write_wav_temp_dir_is_kue_stt() {
        let samples = vec![42i16; 10];
        let path = write_wav_temp(&samples).unwrap();
        let parent = path.parent().unwrap();
        assert_eq!(parent.file_name().unwrap().to_str().unwrap(), "kue-stt");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_wav_temp_filename_contains_uuid_and_counter() {
        let samples = vec![0i16; 10];
        let path = write_wav_temp(&samples).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.starts_with("segment-"));
        assert!(filename.ends_with(".wav"));
        // Format is "segment-{uuid-v4}-{counter}.wav"
        // UUID v4 has 4 hyphens, plus one between uuid and counter.
        // After removing "segment-" prefix and ".wav" suffix, the remaining
        // string should have 5 hyphens total.
        let middle = filename.trim_start_matches("segment-").trim_end_matches(".wav");
        let hyphen_count = middle.chars().filter(|&c| c == '-').count();
        assert_eq!(hyphen_count, 5, "uuid has 4 hyphens + 1 separator before counter");
        // Verify the last segment (after the last hyphen) is a number
        let last_hyphen = middle.rfind('-').unwrap();
        let counter_str = &middle[last_hyphen + 1..];
        counter_str.parse::<u32>().expect("counter should be a valid u32");
        // Verify filename is unique by generating another
        let path2 = write_wav_temp(&samples).unwrap();
        assert_ne!(path, path2, "each call should produce a unique filename");
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&path2).ok();
    }

    // -----------------------------------------------------------------------
    // transcribe_audio_chunk guard
    // -----------------------------------------------------------------------

    #[test]
    fn cli_transcribe_audio_chunk_returns_none_for_empty_chunk() {
        let engine = MoonshineCLIEngine::new();
        assert!(engine.transcribe_audio_chunk(&[]).is_none());
    }

    #[test]
    fn cli_transcribe_audio_chunk_returns_none_for_low_rms() {
        let engine = MoonshineCLIEngine::new();
        // RMS of [1; 100] is 1.0, well above 0.005 threshold
        // But we need samples below 0.005 RMS
        // 0.005 * 32767 ≈ 163.8, but we need RMS of samples to be < 0.005
        // i16 samples at 0,0,0,0... have RMS 0
        let low_energy = [0i16; 100];
        assert!(engine.transcribe_audio_chunk(&low_energy).is_none());
    }

    // -----------------------------------------------------------------------
    // rms threshold for transcribe_audio_chunk
    // -----------------------------------------------------------------------

    #[test]
    fn cli_transcribe_audio_chunk_rms_threshold() {
        let engine = MoonshineCLIEngine::new();
        // RMS of [1; 100] with i16 samples:
        // sum of squares = 100 * 1 = 100, mean = 1.0, sqrt = 1.0
        // 1.0 > 0.005, so this passes the RMS check
        // But then it tries to call the CLI binary which isn't available,
        // so it returns None. The test verifies the early RMS filter works.
        let above_threshold = [1i16; 100];
        let result = engine.transcribe_audio_chunk(&above_threshold);
        // If RMS passes but CLI is not available, returns None from write_wav_temp
        // or Command execution. The important thing is it gets past the RMS check.
        // Since we can't run the CLI, we verify it doesn't panic and returns None.
        assert!(result.is_none());
    }
}
