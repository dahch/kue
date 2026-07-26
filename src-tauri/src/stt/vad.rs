use crate::stt::rms;

pub struct SimpleVAD {
    threshold: f32,
    #[allow(dead_code)]
    sample_rate: u32,
    min_speech_samples: usize,
    silence_samples: usize,
    speech_samples: usize,
    silence_run: usize,
    in_speech: bool,
}

impl SimpleVAD {
    pub fn new(threshold: f32, sample_rate: u32, min_speech_duration_ms: u64, silence_timeout_ms: u64) -> Self {
        Self {
            threshold,
            sample_rate,
            min_speech_samples: (sample_rate as u64 * min_speech_duration_ms / 1000) as usize,
            silence_samples: (sample_rate as u64 * silence_timeout_ms / 1000) as usize,
            speech_samples: 0,
            silence_run: 0,
            in_speech: false,
        }
    }

    pub fn is_speech(&mut self, samples: &[i16]) -> bool {
        let rms = rms(samples);
        let above = rms > self.threshold;

        if above {
            self.speech_samples += samples.len();
            self.silence_run = 0;
            if !self.in_speech && self.speech_samples >= self.min_speech_samples {
                self.in_speech = true;
            }
        } else {
            self.silence_run += samples.len();
            if self.in_speech && self.silence_run >= self.silence_samples {
                self.in_speech = false;
                self.speech_samples = 0;
            }
        }

        self.in_speech
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.speech_samples = 0;
        self.silence_run = 0;
        self.in_speech = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::rms;

    // -----------------------------------------------------------------------
    // rms
    // -----------------------------------------------------------------------

    #[test]
    fn rms_zero_for_empty() {
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_zero_samples() {
        assert_eq!(rms(&[0, 0, 0]), 0.0);
    }

    #[test]
    fn rms_positive_samples() {
        let r = rms(&[100, 200, 300]);
        assert!(r > 0.0);
        assert!(r < 300.0);
    }

    #[test]
    fn rms_negative_samples() {
        let r = rms(&[-100, -200, -300]);
        assert!(r > 0.0);
        assert!(r < 300.0);
    }

    #[test]
    fn rms_mixed_sign() {
        let r1 = rms(&[-100, 100]);
        let r2 = rms(&[-200, 200]);
        // RMS of [-200, 200] should be higher than [-100, 100]
        assert!(r2 > r1);
    }

    #[test]
    fn rms_max_int16() {
        let r = rms(&[i16::MAX, i16::MAX]);
        assert!(r > 32766.0);
        assert!(r <= 32767.0);
    }

    #[test]
    fn rms_single_sample() {
        assert_eq!(rms(&[50]), 50.0);
    }

    // -----------------------------------------------------------------------
    // SimpleVAD
    // -----------------------------------------------------------------------

    fn make_chunk(value: i16, len: usize) -> Vec<i16> {
        vec![value; len]
    }

    #[test]
    fn vad_starts_silent() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        let chunk = make_chunk(0, 160); // 10ms of silence
        assert!(!vad.is_speech(&chunk));
    }

    #[test]
    fn vad_detects_speech_above_threshold() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // First chunk: 100ms — not enough to trigger `in_speech`
        let chunk = make_chunk(5000, 1600);
        assert!(!vad.is_speech(&chunk), "100ms should be below min_speech_duration of 200ms");
        // Second chunk: another 100ms = 200ms total → should trigger
        let chunk2 = make_chunk(5000, 1600);
        assert!(vad.is_speech(&chunk2), "200ms total should trigger speech");
    }

    #[test]
    fn vad_returns_to_silence_after_timeout() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // Feed enough speech to trigger
        let speech = make_chunk(5000, 3200); // 200ms
        assert!(vad.is_speech(&speech));

        // Feed silence longer than timeout (600ms = 9600 samples)
        let silence = make_chunk(0, 9600);
        assert!(!vad.is_speech(&silence));
    }

    #[test]
    fn vad_reset_clears_state() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // Trigger speech
        let speech = make_chunk(5000, 3200);
        assert!(vad.is_speech(&speech));

        vad.reset();

        // Should be silent again
        let silence = make_chunk(0, 160);
        assert!(!vad.is_speech(&silence));

        // Should need to re-accumulate speech
        let speech2 = make_chunk(5000, 1600);
        assert!(!vad.is_speech(&speech2)); // only 100ms
    }

    #[test]
    fn vad_requires_min_duration() {
        let mut vad = SimpleVAD::new(0.5, 16000, 500, 600); // Need 500ms of speech
        let chunk = make_chunk(i16::MAX, 8000); // 500ms of max volume
        assert!(vad.is_speech(&chunk));
    }

    #[test]
    fn vad_empty_chunk_no_panic() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        assert!(!vad.is_speech(&[]));
    }

    #[test]
    fn vad_threshold_boundary() {
        // Samples right at the threshold
        let mut vad = SimpleVAD::new(0.0, 16000, 200, 600);
        let chunk = make_chunk(0, 3200);
        assert!(!vad.is_speech(&chunk));
    }

    #[test]
    fn vad_tiny_chunks_below_min_duration() {
        let mut vad = SimpleVAD::new(0.01, 16000, 500, 600);
        // 10ms chunks, need 500ms = 50 chunks @ 160 samples each
        for _ in 0..10 {
            let chunk = make_chunk(5000, 160); // 10ms each
            assert!(!vad.is_speech(&chunk), "10 chunks of 10ms = 100ms < 500ms");
        }
        // After 50 chunks total (500ms), trigger
        for i in 10..49 {
            let chunk = make_chunk(5000, 160);
            assert!(!vad.is_speech(&chunk), "chunk {}: still below 500ms", i + 1);
        }
        // 50th chunk (10ms * 50 = 500ms) → should trigger
        let chunk = make_chunk(5000, 160);
        assert!(vad.is_speech(&chunk), "50th chunk should reach 500ms");
    }

    #[test]
    fn vad_speech_accumulates_across_chunks() {
        let mut vad = SimpleVAD::new(0.02, 16000, 400, 600);
        // 4 chunks of 100ms each = 400ms total
        for _ in 0..3 {
            let chunk = make_chunk(5000, 1600); // 100ms each
            assert!(!vad.is_speech(&chunk), "accumulating speech...");
        }
        // 4th chunk should trigger
        let chunk = make_chunk(5000, 1600);
        assert!(vad.is_speech(&chunk));
    }

    #[test]
    fn vad_speech_accumulation_persists_across_silence_before_trigger() {
        // Before speech is triggered, silence does NOT reset accumulation.
        // This is intentional: it prevents false negatives during quiet beginnings.
        let mut vad = SimpleVAD::new(0.02, 16000, 400, 300);
        // 3 chunks of speech (300ms) — still below 400ms threshold
        for _ in 0..3 {
            let chunk = make_chunk(5000, 1600); // 100ms each
            assert!(!vad.is_speech(&chunk));
        }
        // Silence that exceeds the timeout period
        let silence = make_chunk(0, 9600); // >300ms silence
        assert!(!vad.is_speech(&silence));
        // Since in_speech was never true, speech_samples was NOT reset.
        // 1 more chunk (100ms) + existing 300ms = 400ms → triggers speech
        let chunk = make_chunk(5000, 1600);
        assert!(vad.is_speech(&chunk), "accumulated 300ms + 100ms = 400ms should trigger");
    }

    #[test]
    fn vad_silence_resets_accumulation_after_speech_triggered() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 300);
        // Trigger speech (200ms)
        let speech = make_chunk(5000, 3200);
        assert!(vad.is_speech(&speech));
        // Then silence > 300ms should exit speech and reset speech_samples
        let silence = make_chunk(0, 9600); // 600ms silence
        assert!(!vad.is_speech(&silence), "should exit speech after timeout");
        // Now need to re-accumulate from scratch
        for _ in 0..1 {
            let chunk = make_chunk(5000, 1600); // 100ms
            assert!(!vad.is_speech(&chunk), "re-accumulating 100ms < 200ms");
        }
        let chunk = make_chunk(5000, 1600); // another 100ms = 200ms
        assert!(vad.is_speech(&chunk), "200ms after reset should trigger speech");
    }

    #[test]
    fn vad_negative_samples_above_threshold() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // Negative samples with high amplitude should still be detected
        let chunk = make_chunk(-5000, 3200); // 200ms of negative amplitude
        assert!(vad.is_speech(&chunk));
    }

    #[test]
    fn vad_large_chunk_immediately_triggers() {
        let mut vad = SimpleVAD::new(0.02, 16000, 100, 600);
        // A single chunk > min_speech_duration should immediately trigger
        let chunk = make_chunk(5000, 3200); // 200ms > 100ms minimum
        assert!(vad.is_speech(&chunk));
    }

    #[test]
    fn vad_zero_threshold_with_silence() {
        let mut vad = SimpleVAD::new(0.0, 16000, 200, 600);
        // With threshold = 0.0, even silent samples have RMS = 0.0
        // 0.0 > 0.0 is false, so still silent
        let chunk = make_chunk(0, 3200);
        assert!(!vad.is_speech(&chunk));
    }

    #[test]
    fn vad_high_threshold_requires_strong_signal() {
        let mut vad = SimpleVAD::new(10000.0, 16000, 200, 600);
        // RMS of samples [100; 3200] = 100.0, which is < 10000
        let chunk = make_chunk(100, 3200);
        assert!(!vad.is_speech(&chunk), "100 RMS < 10000 threshold");

        // RMS of samples [20000; 3200] = 20000, which is > 10000
        let chunk2 = make_chunk(20000, 3200);
        assert!(vad.is_speech(&chunk2), "20000 RMS > 10000 threshold");
    }

    #[test]
    fn vad_reset_during_speech_works() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // Enter speech state
        let chunk = make_chunk(5000, 3200);
        assert!(vad.is_speech(&chunk));

        // Reset
        vad.reset();
        assert!(!vad.is_speech(&make_chunk(0, 160)), "should be silent after reset");

        // Must re-accumulate from zero
        let chunk = make_chunk(5000, 1600); // 100ms
        assert!(!vad.is_speech(&chunk));
        let chunk = make_chunk(5000, 1600); // another 100ms = 200ms
        assert!(vad.is_speech(&chunk));
    }

    #[test]
    fn vad_exact_silence_timeout_boundary() {
        let mut vad = SimpleVAD::new(0.02, 16000, 200, 600);
        // Trigger speech
        let chunk = make_chunk(5000, 3200);
        assert!(vad.is_speech(&chunk));

        // Silence at exactly boundary: 600ms = 9600 samples @ 16kHz
        // At exactly 9600 samples, silence_run >= silence_samples should trigger
        let silence = make_chunk(0, 9600);
        assert!(!vad.is_speech(&silence), "should exit speech after 600ms silence");

        // Continue silence should stay silent
        let more_silence = make_chunk(0, 1600);
        assert!(!vad.is_speech(&more_silence));
    }
}
