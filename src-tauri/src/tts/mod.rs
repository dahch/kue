use std::process::Command;
use std::sync::{Arc, Mutex};

const TTS_TIMEOUT_SECS: u64 = 30;

/// Voice to use for TTS. Samantha is a high-quality American English female
/// voice available on all macOS versions. It speaks English natively, which
/// is what the STT engine (Moonshine, English-only) expects.
const TTS_VOICE: &str = "Samantha";

/// Speaks `text` aloud using the macOS `say` command with a native English
/// voice so the STT engine can understand the playback clearly.
/// Runs the subprocess with a timeout to prevent hanging indefinitely.
pub fn speak(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }

    let child = Command::new("say")
        .arg("-v")
        .arg(TTS_VOICE)
        .arg(text)
        .spawn()
        .map_err(|e| format!("Failed to spawn 'say' command: {e}"))?;

    let child = Arc::new(Mutex::new(child));
    let child_clone = Arc::clone(&child);

    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<std::process::ExitStatus>>();
    std::thread::spawn(move || {
        let result = child_clone.lock().unwrap().wait();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(std::time::Duration::from_secs(TTS_TIMEOUT_SECS)) {
        Ok(Ok(status)) => {
            if status.success() {
                Ok(())
            } else {
                Err(format!("'say' process exited with status: {status}"))
            }
        }
        Ok(Err(e)) => Err(format!("'say' process wait failed: {e}")),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.lock().unwrap().kill();
            Err(format!("'say' process timed out after {}s", TTS_TIMEOUT_SECS))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("'say' process monitor disconnected unexpectedly".to_string())
        }
    }
}

/// Returns `true` if the `say` binary is available on the system.
/// On macOS this is always available at `/usr/bin/say`.
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
        assert!(result.is_ok(), "say should succeed on macOS: {:?}", result.err());
    }

    #[test]
    fn is_available_on_macos() {
        // On macOS, 'say' should always be available at /usr/bin/say
        assert!(is_available(), "'say' command should be available on macOS");
    }
}
