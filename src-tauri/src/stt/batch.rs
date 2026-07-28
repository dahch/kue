use std::path::Path;

use crate::db::Database;
use crate::types::{Speaker, TranscriptLine};

use super::{persist_transcript_line, SimpleVAD, STTConfig, STTEngine};

fn chunk_size(sample_rate: u32) -> usize {
    (sample_rate as usize * 100) / 1000
}

fn sample_offset_to_ms(offset: usize, sample_rate: u32) -> u64 {
    (offset as u64 * 1000) / sample_rate as u64
}

pub fn transcribe_channel_batch(
    wav_path: &Path,
    speaker: Speaker,
    session_id: &str,
    db: &Database,
    engine: &dyn STTEngine,
    config: &STTConfig,
) -> Result<Vec<TranscriptLine>, String> {
    let mut reader =
        hound::WavReader::open(wav_path)
            .map_err(|e| format!("Failed to open WAV at {:?}: {}", wav_path, e))?;

    let samples: Vec<i16> = reader
        .samples::<i16>()
        .filter_map(|s| s.ok())
        .collect();

    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let mut vad = SimpleVAD::new(
        config.vad_threshold,
        config.sample_rate,
        config.min_speech_duration_ms,
        config.silence_timeout_ms,
    );

    let max_segment_samples =
        (config.sample_rate as u64 * config.max_segment_duration_ms / 1000) as usize;

    let mut transcript_lines: Vec<TranscriptLine> = Vec::new();
    let mut speech_buffer: Vec<i16> = Vec::new();
    let mut segment_start_sample: usize = 0;
    let mut in_segment = false;

    let step = chunk_size(config.sample_rate);

    let mut pos = 0usize;
    while pos < samples.len() {
        let end = std::cmp::min(pos + step, samples.len());
        let chunk = &samples[pos..end];

        let speaking = vad.is_speech(chunk);

        if speaking {
            if !in_segment {
                in_segment = true;
                segment_start_sample = pos;
                speech_buffer.clear();
            }
            speech_buffer.extend_from_slice(chunk);

            // Hard cap: if the buffer exceeds max_segment_duration_ms worth
            // of samples, flush it now without waiting for silence. Keep
            // in_segment=true and start a fresh sub-segment.
            if speech_buffer.len() >= max_segment_samples {
                let started_at_ms = sample_offset_to_ms(segment_start_sample, config.sample_rate);
                let ended_at_ms = sample_offset_to_ms(pos + chunk.len(), config.sample_rate);

                if let Some(text) = engine.transcribe_audio_chunk(&speech_buffer) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        persist_transcript_line(
                            db,
                            session_id,
                            trimmed,
                            &speaker,
                            started_at_ms,
                            ended_at_ms,
                        );
                        transcript_lines.push(TranscriptLine {
                            speaker,
                            text: trimmed.to_string(),
                            started_at_ms,
                            ended_at_ms,
                        });
                    }
                }

                speech_buffer.clear();
                // Start a new sub-segment from the current position.
                segment_start_sample = pos + chunk.len();
            }
        } else if in_segment && !speech_buffer.is_empty() {
            let started_at_ms = sample_offset_to_ms(segment_start_sample, config.sample_rate);
            let ended_at_ms = sample_offset_to_ms(pos, config.sample_rate);

            if let Some(text) = engine.transcribe_audio_chunk(&speech_buffer) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    persist_transcript_line(
                        db,
                        session_id,
                        trimmed,
                        &speaker,
                        started_at_ms,
                        ended_at_ms,
                    );
                    transcript_lines.push(TranscriptLine {
                        speaker,
                        text: trimmed.to_string(),
                        started_at_ms,
                        ended_at_ms,
                    });
                }
            }

            speech_buffer.clear();
            in_segment = false;
        }

        pos = end;
    }

    if in_segment && !speech_buffer.is_empty() {
        let started_at_ms = sample_offset_to_ms(segment_start_sample, config.sample_rate);
        let ended_at_ms = sample_offset_to_ms(samples.len(), config.sample_rate);

        if let Some(text) = engine.transcribe_audio_chunk(&speech_buffer) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                persist_transcript_line(db, session_id, trimmed, &speaker, started_at_ms, ended_at_ms);
                transcript_lines.push(TranscriptLine {
                    speaker,
                    text: trimmed.to_string(),
                    started_at_ms,
                    ended_at_ms,
                });
            }
        }
    }

    log::info!(
        "Batch transcription ({}): {} lines",
        speaker.as_db_str(),
        transcript_lines.len()
    );
    Ok(transcript_lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use rusqlite::Connection;

    struct MockEngine {
        result: Option<String>,
    }

    impl STTEngine for MockEngine {
        fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
            Ok(())
        }

        fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
            self.result.clone()
        }
    }

    struct SilentEngine;

    impl STTEngine for SilentEngine {
        fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
            Ok(())
        }

        fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
            None
        }
    }

    fn create_test_db(session_id: &str) -> Database {
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

        Database {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        }
    }

    fn make_speech_chunk(value: i16, duration_ms: u64) -> Vec<i16> {
        let len = (16_000 * duration_ms / 1000) as usize;
        vec![value; len]
    }

    fn write_test_wav(path: &Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }

    // ── Empty / missing WAV ──

    #[test]
    fn batch_empty_wav_returns_no_lines() {
        let dir = std::env::temp_dir().join("kue-batch-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("empty.wav");
        write_test_wav(&wav_path, &[]);

        let engine = MockEngine {
            result: Some("should not appear".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-empty");
        let result =
            transcribe_channel_batch(&wav_path, Speaker::User, "sess-empty", &db, &engine, &config)
                .unwrap();

        assert!(
            result.is_empty(),
            "empty WAV should produce no transcript lines"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_missing_wav_returns_err() {
        let engine = MockEngine {
            result: Some("unused".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-missing");
        let result = transcribe_channel_batch(
            Path::new("/nonexistent/kue-batch-test.wav"),
            Speaker::User,
            "sess-missing",
            &db,
            &engine,
            &config,
        );
        assert!(result.is_err());
    }

    // ── Speaker = User ──

    #[test]
    fn batch_user_speaker_persisted_correctly() {
        let dir = std::env::temp_dir().join("kue-batch-test-user");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("user.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("my answer".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-user");
        let result =
            transcribe_channel_batch(&wav_path, Speaker::User, "sess-user", &db, &engine, &config)
                .unwrap();

        assert_eq!(result.len(), 1, "should have one line");
        let line = &result[0];
        assert_eq!(line.text, "my answer");

        // Verify persistence in DB with speaker = 'user'
        let speaker = {
            let conn = db.conn.lock().unwrap();
            conn.query_row::<String, _, _>(
                "SELECT speaker FROM transcript_lines LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(speaker, "user");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Speaker = Interviewer (batch also works for Channel B if needed) ──

    #[test]
    fn batch_interviewer_speaker_persisted_correctly() {
        let dir = std::env::temp_dir().join("kue-batch-test-interviewer");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("interviewer.wav");

        let speech = make_speech_chunk(5000, 400);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("a question".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-interviewer");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::Interviewer,
            "sess-interviewer",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert_eq!(result[0].text, "a question");
        let speaker = {
            let conn = db.conn.lock().unwrap();
            conn.query_row::<String, _, _>(
                "SELECT speaker FROM transcript_lines LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(speaker, "interviewer");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Multiple segments ──

    #[test]
    fn batch_multiple_segments() {
        let dir = std::env::temp_dir().join("kue-batch-test-multi");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("multi.wav");

        // 300ms speech, 800ms silence, 300ms speech
        let mut samples: Vec<i16> = Vec::new();
        samples.extend_from_slice(&make_speech_chunk(5000, 300));
        samples.extend_from_slice(&make_speech_chunk(0, 800));
        samples.extend_from_slice(&make_speech_chunk(5000, 300));

        write_test_wav(&wav_path, &samples);

        let engine = MockEngine {
            result: Some("response text".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-multi");
        let result =
            transcribe_channel_batch(&wav_path, Speaker::User, "sess-multi", &db, &engine, &config)
                .unwrap();

        assert_eq!(result.len(), 2, "two speech segments should produce two lines");
        assert_eq!(result[0].text, "response text");
        assert_eq!(result[1].text, "response text");
        // Second line should start after the first + silence gap
        assert!(
            result[1].started_at_ms > result[0].ended_at_ms,
            "second segment should start after first ended"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Silent engine (no transcription) ──

    #[test]
    fn batch_silent_engine_returns_no_lines() {
        let dir = std::env::temp_dir().join("kue-batch-test-silent");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("silent.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = SilentEngine;
        let config = STTConfig::default();
        let db = create_test_db("sess-silent");
        let result =
            transcribe_channel_batch(&wav_path, Speaker::User, "sess-silent", &db, &engine, &config)
                .unwrap();

        assert!(result.is_empty(), "silent engine should produce no lines");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Whitespace-only text is filtered ──

    #[test]
    fn batch_whitespace_only_text_filtered() {
        let dir = std::env::temp_dir().join("kue-batch-test-ws");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("whitespace.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("   \n\t  ".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-ws");
        let result =
            transcribe_channel_batch(&wav_path, Speaker::User, "sess-ws", &db, &engine, &config)
                .unwrap();

        assert!(
            result.is_empty(),
            "whitespace-only transcriptions should not be persisted"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── send trait ──

    #[test]
    fn mock_engine_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<MockEngine>();
    }

    // ── sample_offset_to_ms ──

    #[test]
    fn sample_offset_to_ms_zero() {
        assert_eq!(sample_offset_to_ms(0, 16_000), 0);
    }

    #[test]
    fn sample_offset_to_ms_one_second() {
        assert_eq!(sample_offset_to_ms(16_000, 16_000), 1000);
    }

    #[test]
    fn sample_offset_to_ms_half_second() {
        assert_eq!(sample_offset_to_ms(8_000, 16_000), 500);
    }

    // ── chunk_size ──

    #[test]
    fn chunk_size_standard_rate() {
        assert_eq!(chunk_size(16_000), 1_600);
    }

    #[test]
    fn chunk_size_half_rate() {
        assert_eq!(chunk_size(8_000), 800);
    }

    #[test]
    fn chunk_size_cd_rate() {
        assert_eq!(chunk_size(44_100), 4_410);
    }

    #[test]
    fn chunk_size_zero_rate() {
        assert_eq!(chunk_size(0), 0);
    }

    // ── sample_offset_to_ms additional ──

    #[test]
    fn sample_offset_to_ms_large_value() {
        // 100 seconds at 16kHz = 1,600,000 samples → 100,000ms
        assert_eq!(sample_offset_to_ms(1_600_000, 16_000), 100_000);
    }

    #[test]
    fn sample_offset_to_ms_nonstandard_rate() {
        // 44,100 samples at 44.1kHz = 1 second = 1,000ms
        assert_eq!(sample_offset_to_ms(44_100, 44_100), 1_000);
    }

    // ── All-silence WAV ──

    #[test]
    fn batch_all_silence_returns_no_lines() {
        let dir = std::env::temp_dir().join("kue-batch-test-all-silence");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("all_silence.wav");

        // All-zero samples: VAD will never trigger speech
        let silence = make_speech_chunk(0, 3000);
        write_test_wav(&wav_path, &silence);

        let engine = MockEngine {
            result: Some("unreachable".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-all-silence");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-all-silence",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(result.is_empty(), "all-silence WAV should produce no lines");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Engine returns empty string ──

    #[test]
    fn batch_engine_returns_empty_string_filtered() {
        let dir = std::env::temp_dir().join("kue-batch-test-empty-str");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("empty_str.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some(String::new()), // empty string, not just whitespace
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-empty-str");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-empty-str",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(result.is_empty(), "empty-string transcription should be filtered");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Mixed valid and whitespace segments ──

    #[test]
    fn batch_mixed_valid_and_whitespace_segments() {
        let dir = std::env::temp_dir().join("kue-batch-test-mixed");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("mixed.wav");

        // First speech segment
        let mut samples: Vec<i16> = Vec::new();
        samples.extend_from_slice(&make_speech_chunk(5000, 300));
        // Silence gap
        samples.extend_from_slice(&make_speech_chunk(0, 800));
        // Second speech segment
        samples.extend_from_slice(&make_speech_chunk(5000, 300));

        write_test_wav(&wav_path, &samples);

        // Custom engine that returns different texts per call
        struct TwoResultEngine {
            results: Vec<Option<String>>,
            call_count: Mutex<usize>,
        }

        impl STTEngine for TwoResultEngine {
            fn load(&mut self, _model_path: &PathBuf, _language: &str) -> Result<(), String> {
                Ok(())
            }

            fn transcribe_audio_chunk(&self, _chunk: &[i16]) -> Option<String> {
                let mut count = self.call_count.lock().unwrap();
                let res = self.results.get(*count).cloned().flatten();
                *count += 1;
                res
            }
        }

        let engine = TwoResultEngine {
            results: vec![
                Some("valid text".into()),             // first segment: valid
                Some("   \n\t  ".into()),              // second segment: whitespace only
            ],
            call_count: Mutex::new(0),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-mixed");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-mixed",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert_eq!(result.len(), 1, "only the valid segment should be kept");
        assert_eq!(result[0].text, "valid text");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Trailing segment with whitespace transcription ──

    #[test]
    fn batch_trailing_segment_whitespace_filtered() {
        let dir = std::env::temp_dir().join("kue-batch-test-trailing-ws");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("trailing_ws.wav");

        // Speech at the very end of the WAV (no trailing silence)
        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("  \t  ".into()), // whitespace only
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-trailing-ws");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-trailing-ws",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(
            result.is_empty(),
            "whitespace-only trailing segment should be filtered"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Corrupt WAV ──

    #[test]
    fn batch_corrupt_wav_returns_err() {
        let dir = std::env::temp_dir().join("kue-batch-test-corrupt");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("corrupt.wav");

        // Write random bytes that are not a valid WAV header
        std::fs::write(&wav_path, b"this is not a valid RIFF/WAV file").unwrap();

        let engine = MockEngine {
            result: Some("unused".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-corrupt");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-corrupt",
            &db,
            &engine,
            &config,
        );

        assert!(result.is_err(), "corrupt WAV should produce an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Speech too short for VAD ──

    #[test]
    fn batch_speech_too_short_for_vad() {
        let dir = std::env::temp_dir().join("kue-batch-test-short");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("short.wav");

        // 100ms of speech — below the default min_speech_duration
        let speech = make_speech_chunk(5000, 100);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("should not appear".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-short");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-short",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(
            result.is_empty(),
            "speech below VAD min duration should produce no lines"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Partial final chunk (uneven samples) ──

    #[test]
    fn batch_uneven_samples_partial_chunk() {
        let dir = std::env::temp_dir().join("kue-batch-test-uneven");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("uneven.wav");

        // 250ms at 16kHz = 4000 samples. chunk_size = 1600.
        // 4000 / 1600 = 2 full chunks + 800-sample partial chunk
        let speech = make_speech_chunk(5000, 250);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("partial chunk text".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-uneven");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-uneven",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert_eq!(result.len(), 1, "should still transcribe with partial final chunk");
        assert_eq!(result[0].text, "partial chunk text");
        // 250ms → start at 0, end near 250ms
        assert!(result[0].ended_at_ms >= 200 && result[0].ended_at_ms <= 300);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Different sample rate ──

    #[test]
    fn batch_different_sample_rate() {
        let dir = std::env::temp_dir().join("kue-batch-test-8k");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("rate8k.wav");

        // 300ms at 8kHz = 2400 samples
        let len = (8_000 * 300 / 1000) as usize;
        let samples: Vec<i16> = vec![5000; len];
        // Write WAV at 8kHz
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav_path, spec).unwrap();
        for &s in &samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();

        let engine = MockEngine {
            result: Some("low sample rate".into()),
        };
        let mut config = STTConfig::default();
        config.sample_rate = 8_000;
        let db = create_test_db("sess-8k");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-8k",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert_eq!(result.len(), 1, "should transcribe at 8kHz sample rate");
        assert_eq!(result[0].text, "low sample rate");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── DB persistence: timestamps match ──

    #[test]
    fn batch_db_timestamps_match_returned_values() {
        let dir = std::env::temp_dir().join("kue-batch-test-ts");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("ts_check.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("timestamp check".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-ts");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-ts",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        let line = &result[0];
        let (db_start, db_end): (u64, u64) = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT started_at_ms, ended_at_ms FROM transcript_lines LIMIT 1",
                [],
                |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)),
            )
            .unwrap()
        };

        assert_eq!(db_start, line.started_at_ms, "DB started_at_ms should match");
        assert_eq!(db_end, line.ended_at_ms, "DB ended_at_ms should match");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── DB persistence: session_id ──

    #[test]
    fn batch_db_session_id_correct() {
        let dir = std::env::temp_dir().join("kue-batch-test-sid");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("sid_check.wav");

        let speech = make_speech_chunk(5000, 300);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("session check".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-target-id");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-target-id",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(!result.is_empty());
        let db_session_id: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT session_id FROM transcript_lines LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(db_session_id, "sess-target-id");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Max segment duration: long speech without silence ──

    #[test]
    fn batch_long_continuous_speech_splits_into_multiple_segments() {
        let dir = std::env::temp_dir().join("kue-batch-test-long");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("long.wav");

        // 20s of continuous speech (all above VAD threshold, no silence gaps)
        let speech = make_speech_chunk(5000, 20_000);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("long segment text".into()),
        };
        let mut config = STTConfig::default();
        config.max_segment_duration_ms = 12_000; // explicit
        let db = create_test_db("sess-long");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-long",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert!(
            result.len() >= 2,
            "20s of continuous speech should produce at least 2 segments (max 12s each), got {}",
            result.len()
        );
        // Verify DB has the same count
        let conn = db.conn.lock().unwrap();
        let db_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcript_lines", [], |row| row.get(0))
            .unwrap();
        assert_eq!(db_count as usize, result.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_short_speech_below_max_duration_stays_single_segment() {
        let dir = std::env::temp_dir().join("kue-batch-test-short-ok");
        let _ = std::fs::create_dir_all(&dir);
        let wav_path = dir.join("short_ok.wav");

        // 5s of speech (well below 12s max)
        let speech = make_speech_chunk(5000, 5_000);
        write_test_wav(&wav_path, &speech);

        let engine = MockEngine {
            result: Some("short answer".into()),
        };
        let config = STTConfig::default();
        let db = create_test_db("sess-short-ok");
        let result = transcribe_channel_batch(
            &wav_path,
            Speaker::User,
            "sess-short-ok",
            &db,
            &engine,
            &config,
        )
        .unwrap();

        assert_eq!(
            result.len(),
            1,
            "5s of speech (below 12s max) should remain a single segment"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
