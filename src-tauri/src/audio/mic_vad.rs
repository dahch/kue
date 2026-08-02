use std::time::Instant;

use crate::stt::SimpleVAD;

/// Thread-safe shared state for detecting user speech on mic (Channel A).
///
/// Wraps a `SimpleVAD` and tracks the timestamp of every silence→speech
/// transition so that the hint worker can ask "has the user started speaking
/// since time T?" before emitting a Shadow-mode hint.
pub struct MicVadState {
    vad: SimpleVAD,
    was_speaking: bool,
    last_speech_start: Option<Instant>,
}

impl MicVadState {
    pub fn new() -> Self {
        Self {
            // Same default params as the STT pipeline's VAD:
            //   threshold=0.02, 16kHz,  200ms min speech, 600ms silence timeout
            vad: SimpleVAD::new(0.02, 16_000, 200, 600),
            was_speaking: false,
            last_speech_start: None,
        }
    }

    /// Feed a chunk of i16 mic audio samples and update VAD state.
    /// Must be called from the mic VAD monitor thread.
    pub fn feed_audio(&mut self, samples: &[i16]) {
        let now_speaking = self.vad.is_speech(samples);
        if now_speaking && !self.was_speaking {
            self.last_speech_start = Some(Instant::now());
        }
        self.was_speaking = now_speaking;
    }

    /// Returns `true` if the VAD detected a silence→speech transition after
    /// `since`.  This is the primary signal the hint worker uses to decide
    /// whether to cancel a pending Shadow hint.
    pub fn has_speech_since(&self, since: Instant) -> bool {
        self.last_speech_start.is_some_and(|t| t > since)
    }

    /// Returns `true` if the VAD currently considers the user to be speaking.
    /// Useful for cancelling hints when the user was already in the middle of
    /// a response before the question finished being classified.
    pub fn is_currently_speaking(&self) -> bool {
        self.was_speaking
    }

    /// Reset all state (VAD + transition tracking).  Called when the mic
    /// capture starts, so that speech from a previous session doesn't carry
    /// over.
    pub fn reset(&mut self) {
        self.vad.reset();
        self.was_speaking = false;
        self.last_speech_start = None;
    }
}

impl Default for MicVadState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_chunk(value: i16, len: usize) -> Vec<i16> {
        vec![value; len]
    }

    fn silence_chunk() -> Vec<i16> {
        make_chunk(0, 160) // 10ms  @ 16kHz
    }

    fn speech_chunk() -> Vec<i16> {
        make_chunk(5000, 1600) // 100ms @ 16kHz, above threshold
    }

    // ── Initial state ──

    #[test]
    fn starts_silent() {
        let s = MicVadState::new();
        assert!(!s.is_currently_speaking());
        assert!(!s.has_speech_since(Instant::now()));
    }

    // ── feed_audio: speech detection ──

    #[test]
    fn feed_silence_does_not_trigger() {
        let mut s = MicVadState::new();
        let before = Instant::now();
        s.feed_audio(&silence_chunk());
        assert!(!s.is_currently_speaking());
        assert!(!s.has_speech_since(before));
    }

    #[test]
    fn feed_speech_triggers_after_min_duration() {
        let mut s = MicVadState::new();
        let before = Instant::now();
        // 200ms of speech (2 x 100ms) to hit min_speech_duration
        s.feed_audio(&speech_chunk()); // 100ms — not enough yet
        assert!(!s.is_currently_speaking());
        s.feed_audio(&speech_chunk()); // another 100ms = 200ms → trigger
        assert!(s.is_currently_speaking());
        assert!(s.has_speech_since(before));
    }

    #[test]
    fn feed_speech_detects_transition() {
        let mut s = MicVadState::new();
        let before = Instant::now();
        // Accumulate to trigger
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());
        assert!(
            s.has_speech_since(before),
            "should have recorded a speech start"
        );

        // After a transition, has_speech_since should still return true for
        // a time BEFORE the transition.
        assert!(
            s.has_speech_since(before - Duration::from_secs(1)),
            "speech started after the ancient time"
        );
        // And false for a time AFTER the transition.
        let after = Instant::now() + Duration::from_secs(60);
        assert!(!s.has_speech_since(after), "no speech in the far future");
    }

    #[test]
    fn multiple_transitions_keep_latest() {
        let mut s = MicVadState::new();
        // First speech segment
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());
        let first_start = s.last_speech_start;

        // Go silent past the timeout
        for _ in 0..100 {
            s.feed_audio(&silence_chunk()); // 10ms x 100 = 1s > 600ms timeout
        }
        assert!(!s.is_currently_speaking());

        // Second speech segment
        std::thread::sleep(Duration::from_millis(5));
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());

        // last_speech_start should have been updated
        let second_start = s.last_speech_start;
        assert!(
            second_start > first_start,
            "second speech start should be later than first"
        );
    }

    // ── has_speech_since: edge cases ──

    #[test]
    fn has_speech_since_false_when_no_speech_ever() {
        let s = MicVadState::new();
        assert!(!s.has_speech_since(Instant::now()));
        assert!(!s.has_speech_since(Instant::now() - Duration::from_secs(60)));
    }

    #[test]
    fn has_speech_since_false_when_speech_before_since() {
        let mut s = MicVadState::new();
        let boundary = Instant::now() + Duration::from_secs(10);
        // Speech happened before boundary
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(
            !s.has_speech_since(boundary),
            "speech started before boundary should not count"
        );
    }

    // ── reset ──

    #[test]
    fn reset_clears_state() {
        let mut s = MicVadState::new();
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());

        s.reset();
        assert!(!s.is_currently_speaking());
        assert!(!s.has_speech_since(Instant::now()));
    }

    #[test]
    fn reset_then_feed_starts_fresh() {
        let mut s = MicVadState::new();
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        s.reset();
        // Must re-accumulate
        let after_reset = Instant::now();
        s.feed_audio(&speech_chunk()); // 100ms < 200ms
        assert!(!s.is_currently_speaking());
        assert!(
            !s.has_speech_since(after_reset),
            "reset cleared last_speech_start"
        );

        s.feed_audio(&speech_chunk()); // now 200ms
        assert!(s.is_currently_speaking());
        assert!(s.has_speech_since(after_reset));
    }

    // ── is_currently_speaking: lifecycle ──

    #[test]
    fn currently_speaking_false_after_silence_timeout() {
        let mut s = MicVadState::new();
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());

        // Silence > timeout (600ms = 9600 samples)
        let long_silence = make_chunk(0, 9600);
        s.feed_audio(&long_silence);
        assert!(!s.is_currently_speaking());
    }

    // ── Default impl ──

    #[test]
    fn default_is_same_as_new() {
        let a = MicVadState::new();
        let b = MicVadState::default();
        assert_eq!(a.is_currently_speaking(), b.is_currently_speaking());
        assert_eq!(
            a.has_speech_since(Instant::now()),
            b.has_speech_since(Instant::now())
        );
    }

    // ── Edge cases ──

    #[test]
    fn feed_audio_empty_slice_no_panic() {
        let mut s = MicVadState::new();
        // An empty slice should not crash or change state
        s.feed_audio(&[]);
        assert!(!s.is_currently_speaking());
        assert_eq!(s.last_speech_start, None);
    }

    #[test]
    fn has_speech_since_equal_timestamp_returns_false() {
        // `has_speech_since` uses strict `>` comparison.  If the speech
        // started *exactly* at `since`, it should return `false`.
        let mut s = MicVadState::new();
        // Trigger speech
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        let speech_start = s.last_speech_start.unwrap();
        // Use the exact same instant
        assert!(
            !s.has_speech_since(speech_start),
            "t == since should not count (strict >)"
        );
    }

    #[test]
    fn reset_is_idempotent() {
        let mut s = MicVadState::new();
        // Put into a non-trivial state
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        assert!(s.is_currently_speaking());

        // Reset once
        s.reset();
        let after_first = s.last_speech_start;

        // Reset again — should be no different
        s.reset();
        assert!(!s.is_currently_speaking());
        assert_eq!(s.last_speech_start, after_first);
    }

    #[test]
    fn feed_audio_large_chunk_exceeds_min_duration() {
        let mut s = MicVadState::new();
        let before = Instant::now();
        // A single chunk larger than min_speech_duration (200ms @ 16kHz = 3200 samples)
        let big_chunk = make_chunk(5000, 4800); // 300ms
        s.feed_audio(&big_chunk);
        assert!(s.is_currently_speaking());
        assert!(s.has_speech_since(before));
    }

    #[test]
    fn feed_audio_transition_to_silence_does_not_update_last_speech_start() {
        let mut s = MicVadState::new();
        // Trigger speech
        for _ in 0..2 {
            s.feed_audio(&speech_chunk());
        }
        let start = s.last_speech_start;

        // Silence > timeout should end speech but NOT update last_speech_start
        let long_silence = make_chunk(0, 9600);
        s.feed_audio(&long_silence);
        assert!(!s.is_currently_speaking());
        // last_speech_start should still be the old transition time
        assert_eq!(
            s.last_speech_start, start,
            "transition to silence must not overwrite last_speech_start"
        );
    }

    #[test]
    fn feed_audio_silence_after_speech_keeps_was_speaking_false() {
        let mut s = MicVadState::new();

        // Start silent
        s.feed_audio(&silence_chunk());
        assert!(!s.is_currently_speaking());
        assert_eq!(s.last_speech_start, None);

        // More silence should stay silent and not set last_speech_start
        let big_silence = make_chunk(0, 9600);
        s.feed_audio(&big_silence);
        assert!(!s.is_currently_speaking());
        assert_eq!(s.last_speech_start, None);
    }

    // ── Concurrency / thread-safety ──

    /// MicVadState's methods take `&mut self` so it's not `Sync`, but it must
    /// be `Send` so it can be placed behind a `Mutex` and sent between threads.
    #[test]
    fn mic_vad_state_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MicVadState>();
    }
}
