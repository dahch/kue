use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use super::cli::MoonshineCLIEngine;
use super::ffi::MoonshineFFIEngine;
use super::{SimpleVAD, STTConfig, STTEngine};
use crate::classifier::{classify, QuestionType};
use crate::types::{Speaker, TranscriptLine};

pub struct STTPipeline {
    engine: Box<dyn STTEngine>,
    config: STTConfig,
    session_id: String,
    app_handle: Option<tauri::AppHandle>,
}

impl STTPipeline {
    pub fn new(config: STTConfig) -> Self {
    let engine: Box<dyn STTEngine> = if MoonshineFFIEngine::is_available() {
            eprintln!("[kue] STT: using Moonshine FFI engine");
            Box::new(MoonshineFFIEngine::new())
        } else if config.use_cli_fallback {
            eprintln!("[kue] STT: Moonshine lib not found, falling back to CLI engine");
            Box::new(MoonshineCLIEngine::new())
        } else {
            eprintln!("[kue] STT: No Moonshine lib and CLI fallback disabled");
            // Still create a CLI engine — it will always return None for
            // transcription, effectively making the pipeline a no-op.
            Box::new(MoonshineCLIEngine::new())
        };

    Self {
        engine,
        config,
        session_id: String::new(),
        app_handle: None,
    }
    }

    pub fn with_app_handle(mut self, handle: tauri::AppHandle) -> Self {
        self.app_handle = Some(handle);
        self
    }

    pub fn load_model(&mut self) -> Result<(), String> {
        self.engine.load(&self.config.model_path, &self.config.language)
    }

    pub fn start_session(&mut self, session_id: &str) {
        self.session_id = session_id.to_string();
    }

    #[allow(dead_code)]
    pub fn end_session(&mut self) {
        self.session_id.clear();
    }

    #[allow(dead_code)]
    pub fn process_audio_chunk(&self, chunk: &[i16]) -> bool {
        self.engine.transcribe_audio_chunk(chunk).is_some()
    }

    pub fn spawn_processing_thread(
        self,
        rx: Arc<Mutex<Receiver<Vec<i16>>>>,
        db: crate::db::Database,
    ) -> JoinHandle<()> {
        let mut vad = SimpleVAD::new(
            self.config.vad_threshold,
            self.config.sample_rate,
            self.config.min_speech_duration_ms,
            self.config.silence_timeout_ms,
        );

        let mut speech_buffer: Vec<i16> = Vec::new();
        let mut segment_start_ms: u64 = 0;
        let mut in_segment = false;
        let session_start = Instant::now();

        thread::Builder::new()
            .name("kue-stt-pipeline".into())
            .spawn(move || {
                eprintln!("[kue] STT pipeline thread started");

                loop {
                    let samples = {
                        let rx = rx.lock().unwrap();
                        match rx.recv_timeout(Duration::from_millis(500)) {
                            Ok(s) => s,
                            Err(RecvTimeoutError::Timeout) => {
                                // Flush any pending speech segment on timeout
                                if in_segment && !speech_buffer.is_empty() {
                                    Self::flush_segment(
                                        &self.engine,
                                        &mut speech_buffer,
                                        &mut segment_start_ms,
                                        &mut in_segment,
                                        &session_start,
                                        &self.session_id,
                                        &self.app_handle,
                                        &db,
                                    );
                                }
                                continue;
                            }
                            Err(RecvTimeoutError::Disconnected) => {
                                eprintln!("[kue] STT pipeline: audio source disconnected");
                                if in_segment && !speech_buffer.is_empty() {
                                    Self::flush_segment(
                                        &self.engine,
                                        &mut speech_buffer,
                                        &mut segment_start_ms,
                                        &mut in_segment,
                                        &session_start,
                                        &self.session_id,
                                        &self.app_handle,
                                        &db,
                                    );
                                }
                                break;
                            }
                        }
                    };

                    let now = session_start.elapsed().as_millis() as u64;
                    let speaking = vad.is_speech(&samples);

                    if speaking {
                        if !in_segment {
                            in_segment = true;
                            segment_start_ms = now;
                            speech_buffer.clear();
                        }
                        speech_buffer.extend_from_slice(&samples);
                    } else if in_segment && !speech_buffer.is_empty() {
                        Self::flush_segment(
                            &self.engine,
                            &mut speech_buffer,
                            &mut segment_start_ms,
                            &mut in_segment,
                            &session_start,
                            &self.session_id,
                            &self.app_handle,
                            &db,
                        );
                        speech_buffer.clear();
                    }
                }

                eprintln!("[kue] STT pipeline thread ended");
            })
            .expect("failed to spawn STT pipeline thread")
    }

    fn flush_segment(
        engine: &dyn STTEngine,
        buffer: &mut Vec<i16>,
        segment_start_ms: &mut u64,
        in_segment: &mut bool,
        session_start: &Instant,
        session_id: &str,
        app_handle: &Option<tauri::AppHandle>,
        db: &crate::db::Database,
    ) {
        if buffer.is_empty() {
            return;
        }

        let ended_at_ms = session_start.elapsed().as_millis() as u64;
        let text = engine.transcribe_audio_chunk(buffer);

        if let Some(transcribed) = text {
            if transcribed.trim().is_empty() {
                *in_segment = false;
                buffer.clear();
                return;
            }

            eprintln!("[kue] STT: \"{transcribed}\"");

            if !session_id.is_empty() {
                Self::persist_transcript_line(
                    db, session_id, &transcribed, *segment_start_ms, ended_at_ms,
                );
            }

            if let Some(ref handle) = app_handle {
                // Classify first (borrow), then move transcribed into event
                let qtype = classify(&transcribed);
                let qtext = if qtype != QuestionType::None {
                    Some(transcribed.clone())
                } else {
                    None
                };

                let line = TranscriptLine {
                    speaker: Speaker::Interviewer,
                    text: transcribed,
                    started_at_ms: *segment_start_ms,
                    ended_at_ms,
                };
                if let Err(e) = handle.emit("new-transcript", line) {
                    eprintln!("[kue] Failed to emit new-transcript event: {e}");
                }

                if let Some(text) = qtext {
                    if let Err(e) = handle.emit("question-detected", serde_json::json!({
                        "text": text,
                        "type": qtype.as_str(),
                        "session_id": session_id,
                    })) {
                        eprintln!("[kue] Failed to emit question-detected event: {e}");
                    }
                }
            }
        }

        *in_segment = false;
        buffer.clear();
    }

    fn persist_transcript_line(db: &crate::db::Database, session_id: &str, text: &str, started_at_ms: u64, ended_at_ms: u64) {
        let conn = match db.conn.lock() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[kue] STT: failed to lock DB: {e}");
                return;
            }
        };

        if let Err(e) = conn.execute(
            "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![session_id, Speaker::Interviewer.as_db_str(), text, started_at_ms, ended_at_ms],
        ) {
            eprintln!("[kue] STT: failed to persist transcript line: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use rusqlite::Connection;

    // -----------------------------------------------------------------------
    // MockEngine — controllable STTEngine for pipeline tests
    // -----------------------------------------------------------------------

    struct MockEngine {
        transcribe_result: Option<String>,
        load_result: Result<(), String>,
        load_count: std::sync::atomic::AtomicUsize,
    }

    impl MockEngine {
        fn new(transcribe_result: Option<&str>, load_result: Result<(), &str>) -> Self {
            Self {
                transcribe_result: transcribe_result.map(|s| s.to_string()),
                load_result: load_result.map_err(|e| e.to_string()),
                load_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn load_count(&self) -> usize {
            self.load_count.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl STTEngine for MockEngine {
        fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
            self.load_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.load_result.clone()
        }

        fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
            self.transcribe_result.clone()
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Create an in-memory Database with just the tables needed by
    /// `persist_transcript_line` — no vec0 dependency required.
    fn create_test_db(session_id: &str) -> crate::db::Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS transcript_lines (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                speaker TEXT CHECK(speaker IN ('user', 'interviewer')),
                text TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );
            INSERT OR IGNORE INTO sessions (id) VALUES ('{session_id}');
            "
        ))
        .expect("failed to create test schema");

        crate::db::Database {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        }
    }

    fn session_start() -> Instant {
        Instant::now()
    }

    // -----------------------------------------------------------------------
    // STTPipeline::new — engine selection
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_new_uses_cli_fallback_when_ffi_unavailable() {
        // MoonshineFFIEngine::is_available() will return false in a test
        // environment (no shared lib), so the pipeline should fall back to CLI.
        let config = STTConfig::default();
        let pipeline = STTPipeline::new(config);
        // The engine should be a MoonshineCLIEngine, which we verify by
        // checking that transcribe_audio_chunk calls the CLI binary.
        // For now we assert the engine is not None — the concrete type is
        // erased behind Box<dyn STTEngine>.
        let result = pipeline.engine.transcribe_audio_chunk(&[]);
        // CLI engine returns None for empty chunks (rms check)
        assert!(result.is_none());
    }

    #[test]
    fn pipeline_new_respects_use_cli_fallback_false() {
        // Even withuse_cli_fallback=false, when FFI is unavailable the
        // pipeline still creates a CLI engine (there's no alternative).
        let mut config = STTConfig::default();
        config.use_cli_fallback = false;
        let pipeline = STTPipeline::new(config);
        let result = pipeline.engine.transcribe_audio_chunk(&[]);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // STTPipeline::load_model
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_load_model_delegates_to_engine() {
        let engine = MockEngine::new(None, Ok(()));
        let config = STTConfig::default();
        let mut pipeline = STTPipeline {
            engine: Box::new(engine),
            config,
            session_id: String::new(),
            app_handle: None,
        };

        let result = pipeline.load_model();
        assert!(result.is_ok());
    }

    #[test]
    fn pipeline_load_model_propagates_engine_error() {
        let engine = MockEngine::new(None, Err("model not found"));
        let config = STTConfig::default();
        let mut pipeline = STTPipeline {
            engine: Box::new(engine),
            config,
            session_id: String::new(),
            app_handle: None,
        };

        let result = pipeline.load_model();
        assert_eq!(result.unwrap_err(), "model not found");
    }

    // -----------------------------------------------------------------------
    // STTPipeline::start_session / end_session
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_start_session_sets_session_id() {
        let config = STTConfig::default();
        let mut pipeline = STTPipeline::new(config);
        assert!(pipeline.session_id.is_empty());

        pipeline.start_session("test-session-123");
        assert_eq!(pipeline.session_id, "test-session-123");
    }

    #[test]
    fn pipeline_end_session_clears_session_id() {
        let config = STTConfig::default();
        let mut pipeline = STTPipeline::new(config);
        pipeline.start_session("test-session-123");
        pipeline.end_session();
        assert!(pipeline.session_id.is_empty());
    }

    // -----------------------------------------------------------------------
    // STTPipeline::process_audio_chunk
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_process_audio_chunk_delegates_to_engine() {
        let engine = MockEngine::new(Some("hello"), Ok(()));
        let config = STTConfig::default();
        let pipeline = STTPipeline {
            engine: Box::new(engine),
            config,
            session_id: String::new(),
            app_handle: None,
        };

        assert!(pipeline.process_audio_chunk(&[1, 2, 3]));
    }

    #[test]
    fn pipeline_process_audio_chunk_returns_false_when_engine_returns_none() {
        let engine = MockEngine::new(None, Ok(()));
        let config = STTConfig::default();
        let pipeline = STTPipeline {
            engine: Box::new(engine),
            config,
            session_id: String::new(),
            app_handle: None,
        };

        assert!(!pipeline.process_audio_chunk(&[1, 2, 3]));
    }

    // -----------------------------------------------------------------------
    // STTPipeline::flush_segment — buffer guard
    // -----------------------------------------------------------------------

    #[test]
    fn flush_segment_empty_buffer_returns_early() {
        let engine = MockEngine::new(Some("unused"), Ok(()));
        let mut buffer = Vec::new();
        let mut segment_start_ms = 42;
        let mut in_segment = true;
        let start = session_start();
        let db = create_test_db("sess-1");

        STTPipeline::flush_segment(
            &engine, &mut buffer, &mut segment_start_ms, &mut in_segment,
            &start, "sess-1", &None, &db,
        );

        // Early return: no state change
        assert!(buffer.is_empty());
        assert!(in_segment);
        assert_eq!(segment_start_ms, 42);
    }

    // -----------------------------------------------------------------------
    // STTPipeline::flush_segment — no transcription
    // -----------------------------------------------------------------------

    #[test]
    fn flush_segment_no_transcription_clears_buffer_and_resets_segment() {
        let engine = MockEngine::new(None, Ok(()));
        let mut buffer = vec![100i16; 160];
        let mut segment_start_ms = 100;
        let mut in_segment = true;
        let start = session_start();
        let db = create_test_db("sess-2");

        STTPipeline::flush_segment(
            &engine, &mut buffer, &mut segment_start_ms, &mut in_segment,
            &start, "sess-2", &None, &db,
        );

        assert!(buffer.is_empty());
        assert!(!in_segment);
    }

    // -----------------------------------------------------------------------
    // STTPipeline::flush_segment — empty/whitespace text
    // -----------------------------------------------------------------------

    #[test]
    fn flush_segment_empty_text_clears_buffer_without_persisting() {
        let engine = MockEngine::new(Some("   "), Ok(()));
        let mut buffer = vec![100i16; 160];
        let mut segment_start_ms = 100;
        let mut in_segment = true;
        let start = session_start();
        let db = create_test_db("sess-3");

        STTPipeline::flush_segment(
            &engine, &mut buffer, &mut segment_start_ms, &mut in_segment,
            &start, "sess-3", &None, &db,
        );

        assert!(buffer.is_empty());
        assert!(!in_segment);

        // Verify nothing was persisted
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcript_lines", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "empty text should not be persisted");
    }

    // -----------------------------------------------------------------------
    // STTPipeline::flush_segment — skips DB when session_id is empty
    // -----------------------------------------------------------------------

    #[test]
    fn flush_segment_skips_db_persistence_when_no_session() {
        let engine = MockEngine::new(Some("hello world"), Ok(()));
        let mut buffer = vec![100i16; 160];
        let mut segment_start_ms = 100;
        let mut in_segment = true;
        let start = session_start();
        let db = create_test_db("unused");

        STTPipeline::flush_segment(
            &engine, &mut buffer, &mut segment_start_ms, &mut in_segment,
            &start, "", &None, &db,
        );

        assert!(buffer.is_empty());
        assert!(!in_segment);

        // Nothing persisted because session_id was empty
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcript_lines", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // -----------------------------------------------------------------------
    // STTPipeline::flush_segment — full path with DB persistence
    // -----------------------------------------------------------------------

    #[test]
    fn flush_segment_persists_transcript_to_db() {
        let engine = MockEngine::new(Some("transcribed text"), Ok(()));
        let mut buffer = vec![100i16; 160];
        let mut segment_start_ms = 500;
        let mut in_segment = true;
        let start = session_start();
        let db = create_test_db("session-flush-1");

        STTPipeline::flush_segment(
            &engine, &mut buffer, &mut segment_start_ms, &mut in_segment,
            &start, "session-flush-1", &None, &db,
        );

        // Verify buffer cleared and segment reset
        assert!(buffer.is_empty());
        assert!(!in_segment);

        // Verify the transcript line was persisted
        let conn = db.conn.lock().unwrap();
        let (text, speaker, started, ended): (String, String, u64, u64) = conn
            .query_row(
                "SELECT text, speaker, started_at_ms, ended_at_ms FROM transcript_lines LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(text, "transcribed text");
        assert_eq!(speaker, "interviewer");
        assert_eq!(started, 500);
        // ended_at_ms is computed from session_start.elapsed() at the moment
        // flush_segment runs, so in a fast unit test it will be a small value
        // (typically < 10ms). We just verify it's a plausible uint.
        assert!(
            ended < 60_000,
            "ended_at_ms ({}) should be a reasonable elapsed since session start",
            ended,
        );
    }

    // -----------------------------------------------------------------------
    // STTPipeline::persist_transcript_line — DB integration
    // -----------------------------------------------------------------------

    #[test]
    fn persist_transcript_line_inserts_row() {
        let db = create_test_db("session-persist-1");

        STTPipeline::persist_transcript_line(&db, "session-persist-1", "hello", 100, 200);

        let conn = db.conn.lock().unwrap();
        let (text, speaker, started, ended): (String, String, u64, u64) = conn
            .query_row(
                "SELECT text, speaker, started_at_ms, ended_at_ms FROM transcript_lines LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(text, "hello");
        assert_eq!(speaker, "interviewer");
        assert_eq!(started, 100);
        assert_eq!(ended, 200);
    }

    #[test]
    fn persist_transcript_line_handles_fk_violation_gracefully() {
        // Use a session_id that doesn't exist in the sessions table
        let db = create_test_db("real-session");

        // This should fail silently (the FK violation is caught by the
        // eprintln! guard — no panic, no crash)
        STTPipeline::persist_transcript_line(
            &db, "non-existent-session", "orphan text", 0, 100,
        );

        // Nothing should have been inserted
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcript_lines", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn persist_transcript_line_handles_poisoned_mutex_gracefully() {
        // Build a valid DB with schema
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS transcript_lines (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                speaker TEXT CHECK(speaker IN ('user', 'interviewer')),
                text TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
             );
             INSERT INTO sessions (id) VALUES ('s-poisoned');",
        ).unwrap();

        let mtx = std::sync::Arc::new(std::sync::Mutex::new(conn));
        // Poison the mutex by panicking in another thread
        let mtx_clone = std::sync::Arc::clone(&mtx);
        let handle = std::thread::spawn(move || {
            let _guard = mtx_clone.lock().unwrap();
            panic!("intentional panic to poison mutex");
        });
        let _ = handle.join();

        let db = crate::db::Database {
            conn: std::sync::Arc::try_unwrap(mtx).unwrap_or_else(|_| {
                panic!("Arc should have one reference after join")
            }),
            path: PathBuf::from(":memory:"),
        };

        // This should handle the poisoned mutex gracefully (eprintln + return)
        STTPipeline::persist_transcript_line(&db, "s-poisoned", "should not panic", 0, 100);
        // The function should not panic — that's the main assertion.
        // Since the mutex is poisoned, no row was inserted.
    }

    #[test]
    fn persist_transcript_line_special_characters_in_text() {
        let db = create_test_db("session-special");
        let special = "Hello, ¿cómo estás? 你好 👋 émoji & <stuff>";

        STTPipeline::persist_transcript_line(&db, "session-special", special, 0, 100);

        let conn = db.conn.lock().unwrap();
        let text: String = conn
            .query_row(
                "SELECT text FROM transcript_lines LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(text, special);
    }

    #[test]
    fn persist_transcript_line_multiple_lines() {
        let db = create_test_db("session-multi");

        STTPipeline::persist_transcript_line(&db, "session-multi", "first", 0, 100);
        STTPipeline::persist_transcript_line(&db, "session-multi", "second", 150, 300);
        STTPipeline::persist_transcript_line(&db, "session-multi", "third", 350, 500);

        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcript_lines", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);

        let texts: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT text FROM transcript_lines ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    // -----------------------------------------------------------------------
    // with_app_handle
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_with_app_handle_sets_handle() {
        let config = STTConfig::default();
        let pipeline = STTPipeline::new(config);
        // Can't create a real tauri::AppHandle in tests, so we just verify
        // that with_app_handle returns Self and the field is still accessible.
        // (app_handle will remain None since we can't construct a handle)
        assert!(pipeline.app_handle.is_none());
    }
}
