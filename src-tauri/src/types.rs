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
///
/// # DB contract
/// Values are serialized to lowercase (`"user"`, `"interviewer"`) to match the
/// `CHECK(speaker IN ('user', 'interviewer'))` constraint in the SQL schema.
/// Always use the `Display` impl or `as_db_str()` when persisting.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    User,
    Interviewer,
}

impl Speaker {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Speaker::User => "user",
            Speaker::Interviewer => "interviewer",
        }
    }
}

impl std::fmt::Display for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_db_str())
    }
}

impl From<Speaker> for String {
    fn from(s: Speaker) -> Self {
        s.as_db_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Speaker::as_db_str ──

    #[test]
    fn speaker_user_as_db_str() {
        assert_eq!(Speaker::User.as_db_str(), "user");
    }

    #[test]
    fn speaker_interviewer_as_db_str() {
        assert_eq!(Speaker::Interviewer.as_db_str(), "interviewer");
    }

    // ── Speaker::Display ──

    #[test]
    fn speaker_user_display() {
        assert_eq!(Speaker::User.to_string(), "user");
    }

    #[test]
    fn speaker_interviewer_display() {
        assert_eq!(Speaker::Interviewer.to_string(), "interviewer");
    }

    // ── Speaker::From<Speaker> for String ──

    #[test]
    fn speaker_user_into_string() {
        let s: String = Speaker::User.into();
        assert_eq!(s, "user");
    }

    #[test]
    fn speaker_interviewer_into_string() {
        let s: String = Speaker::Interviewer.into();
        assert_eq!(s, "interviewer");
    }

    // ── Speaker serialization ──

    #[test]
    fn speaker_user_serializes_to_lowercase() {
        let json = serde_json::to_string(&Speaker::User).unwrap();
        assert_eq!(json, r#""user""#);
    }

    #[test]
    fn speaker_interviewer_serializes_to_lowercase() {
        let json = serde_json::to_string(&Speaker::Interviewer).unwrap();
        assert_eq!(json, r#""interviewer""#);
    }

    #[test]
    fn speaker_user_deserializes_from_lowercase() {
        let s: Speaker = serde_json::from_str(r#""user""#).unwrap();
        assert!(matches!(s, Speaker::User));
    }

    #[test]
    fn speaker_interviewer_deserializes_from_lowercase() {
        let s: Speaker = serde_json::from_str(r#""interviewer""#).unwrap();
        assert!(matches!(s, Speaker::Interviewer));
    }

    #[test]
    fn speaker_deserialize_rejects_invalid() {
        let result: Result<Speaker, _> = serde_json::from_str(r#""invalid""#);
        assert!(
            result.is_err(),
            "invalid speaker value should fail to deserialize"
        );
    }

    #[test]
    fn speaker_deserialize_rejects_uppercase() {
        let result: Result<Speaker, _> = serde_json::from_str(r#""User""#);
        assert!(
            result.is_err(),
            "capitalized 'User' should fail (must be lowercase)"
        );
    }

    // ── Speaker Clone + Copy ──

    #[test]
    fn speaker_is_copy() {
        let a = Speaker::User;
        let b = a; // Copy, not move
        assert_eq!(a, b);
    }

    #[test]
    fn speaker_debug_format() {
        let debug = format!("{:?}", Speaker::User);
        assert_eq!(debug, "User");
    }

    // ── TranscriptLine serialization roundtrip ──

    #[test]
    fn transcript_line_serialization_roundtrip() {
        let line = TranscriptLine {
            speaker: Speaker::User,
            text: "Hello, I am a software engineer.".to_string(),
            started_at_ms: 1000,
            ended_at_ms: 3000,
        };

        let json = serde_json::to_string(&line).unwrap();
        let deserialized: TranscriptLine = serde_json::from_str(&json).unwrap();

        assert!(matches!(deserialized.speaker, Speaker::User));
        assert_eq!(deserialized.text, "Hello, I am a software engineer.");
        assert_eq!(deserialized.started_at_ms, 1000);
        assert_eq!(deserialized.ended_at_ms, 3000);
    }

    #[test]
    fn transcript_line_with_empty_text() {
        let line = TranscriptLine {
            speaker: Speaker::Interviewer,
            text: String::new(),
            started_at_ms: 0,
            ended_at_ms: 0,
        };
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: TranscriptLine = serde_json::from_str(&json).unwrap();
        assert!(deserialized.text.is_empty());
    }

    #[test]
    fn transcript_line_with_interviewer() {
        let line = TranscriptLine {
            speaker: Speaker::Interviewer,
            text: "Tell me about yourself".to_string(),
            started_at_ms: 500,
            ended_at_ms: 2500,
        };
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""speaker":"interviewer""#));
        assert!(json.contains(r#""started_at_ms":500"#));
        assert!(json.contains(r#""ended_at_ms":2500"#));
    }

    #[test]
    fn transcript_line_large_timestamps() {
        let line = TranscriptLine {
            speaker: Speaker::User,
            text: "Long answer".to_string(),
            started_at_ms: u64::MAX / 2,
            ended_at_ms: u64::MAX,
        };
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: TranscriptLine = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.started_at_ms, u64::MAX / 2);
        assert_eq!(deserialized.ended_at_ms, u64::MAX);
    }

    #[test]
    fn transcript_line_unicode_text() {
        let line = TranscriptLine {
            speaker: Speaker::User,
            text: "café résumé niño 日本国 📚🧪".to_string(),
            started_at_ms: 0,
            ended_at_ms: 100,
        };
        let json = serde_json::to_string(&line).unwrap();
        let deserialized: TranscriptLine = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.text, "café résumé niño 日本国 📚🧪");
    }
}
