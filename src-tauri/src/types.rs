use serde::{Deserialize, Serialize};

/// A single completed line of transcription.
///
/// This struct is the shared contract between the STT module (Prompt 5),
/// the classifier module (Prompt 6), and the orchestrator (Prompt 7).
/// Both stt/ and classifier/ MUST import this type from `types.rs` rather
/// than redefining their own version — duplicating it would create a
/// brittle coupling that the orchestrator would have to reconcile.
///
/// # Fields
/// - `speaker`: who spoke this line (User or Interviewer).
/// - `text`: the transcribed text (may be empty for silence-only segments).
/// - `started_at_ms`: offset in milliseconds from session start.
/// - `ended_at_ms`: offset in milliseconds from session start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLine {
    pub speaker: Speaker,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

/// Speaker label derived from the audio channel:
/// - `User` = Canal A (micrófono del candidato).
/// - `Interviewer` = Canal B (loopback de sistema, entrevistador).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Speaker {
    User,
    Interviewer,
}
