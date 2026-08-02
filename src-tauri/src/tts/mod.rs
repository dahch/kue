use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const TTS_TIMEOUT_SECS: u64 = 30;
/// Poll interval for checking the cancel flag while TTS is speaking.
const TTS_POLL_INTERVAL_MS: u64 = 50;

/// Voice to use for TTS. Samantha is a high-quality American English female
/// voice available on all macOS versions. It speaks English natively, which
/// is what the STT engine (Moonshine, English-only) expects.
const TTS_VOICE: &str = "Samantha";

/// Speaking rate for `say` (words per minute). Slightly slower than default
/// (~180 WPM) so the interviewer speaks clearly and the STT engine can keep
/// up without missing words.
const TTS_RATE: u16 = 150;

/// Spawns a `say` subprocess and blocks until it finishes, the `cancel` flag
/// is set, or the timeout expires. On cancel or timeout the child process is
/// killed and `Err(msg)` is returned; on natural completion `Ok(())`.
///
/// Cancellation works via the shared `cancel` flag (checked every poll
/// interval) — no lock is held across `wait()`, so the flag setter never
/// blocks on a running child.
pub fn speak_cancellable(text: &str, cancel: Arc<AtomicBool>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty text".to_string());
    }

    let mut child = Command::new("say")
        .arg("-v")
        .arg(TTS_VOICE)
        .arg("-r")
        .arg(TTS_RATE.to_string())
        .arg(text)
        .spawn()
        .map_err(|e| format!("Failed to spawn 'say' command: {e}"))?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(TTS_TIMEOUT_SECS);

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait(); // reap to avoid a zombie process
            return Err("cancelled".to_string());
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait(); // reap to avoid a zombie process
            return Err(format!(
                "'say' process timed out after {}s",
                TTS_TIMEOUT_SECS
            ));
        }

        // Non-blocking poll — does NOT hold a lock across the wait.
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(format!("'say' exited with status: {status}"));
            }
            Ok(None) => {
                // Still running — wait a bit and re-check the flag.
                std::thread::sleep(std::time::Duration::from_millis(TTS_POLL_INTERVAL_MS));
            }
            Err(e) => return Err(format!("'say' wait failed: {e}")),
        }
    }
}

/// Speaks `text` aloud using the macOS `say` command with a native English
/// voice so the STT engine can understand the playback clearly.
/// Runs the subprocess with a timeout to prevent hanging indefinitely.
///
/// Retained as a simple blocking API (used by tests); the interview runner
/// uses `speak_cancellable` for cancellation support. Empty text is a no-op
/// returning `Ok(())`, mirroring `speak_cancellable`'s empty-text guard.
#[allow(dead_code)]
pub fn speak(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    speak_cancellable(text, Arc::new(AtomicBool::new(false)))
}

/// Returns `true` if the `say` binary is available on the system.
/// On macOS this is always available at `/usr/bin/say`.
#[allow(dead_code)]
pub fn is_available() -> bool {
    std::process::Command::new("which")
        .arg("say")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::path::Path::new("/usr/bin/say").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speak_empty_text_returns_ok() {
        assert!(speak("").is_ok());
    }

    #[test]
    fn speak_whitespace_only_returns_ok() {
        assert!(speak("   \n\t ").is_ok());
    }

    #[test]
    #[ignore = "requires macOS 'say' command with audio output"]
    fn speak_short_text_on_macos() {
        let result = speak("Hello from Kue");
        assert!(
            result.is_ok(),
            "say should succeed on macOS: {:?}",
            result.err()
        );
    }

    #[test]
    fn is_available_on_macos() {
        // On macOS, 'say' should always be available at /usr/bin/say
        assert!(is_available(), "'say' command should be available on macOS");
    }
}
