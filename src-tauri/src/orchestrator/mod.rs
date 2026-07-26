use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

pub mod worker;

use crate::classifier::QuestionType;
use crate::db::Database;
use crate::rag::embeddings::Embedder;
use crate::rag::indexer::search;

pub const SHADOW_DELAY_MS: u64 = 2500;
const HINT_TOP_K: usize = 1;

const MAX_HINT_WORDS: usize = 8;

/// A hint generation job sent from the audio pipeline to the hint worker.
#[derive(Debug, Clone)]
pub struct HintJob {
    pub session_id: String,
    pub text: String,
    pub qtype: QuestionType,
    pub mode: String,
}

/// Commands the hint worker can receive from the audio pipeline.
#[derive(Debug, Clone)]
pub enum HintCommand {
    Process(HintJob),
    CancelSession(String),
}

/// Shared sender handle used by `STTPipeline` to enqueue hint jobs
/// without blocking the audio thread.
pub type HintJobSender = Arc<Sender<HintCommand>>;

#[derive(Debug, Clone, Serialize)]
pub struct HintEvent {
    pub text: String,
    #[serde(rename = "type")]
    pub qtype: String,
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub struct PendingHint {
    pub session_id: String,
    pub qtype: QuestionType,
    pub text: String,
    pub fire_at: Instant,
}

pub struct HintScheduler {
    pending: Mutex<Vec<PendingHint>>,
}

impl HintScheduler {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
        }
    }

    pub fn schedule(&self, hint: PendingHint) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(hint);
        }
    }

    pub fn cancel_all(&self, session_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|h| h.session_id != session_id);
        }
    }

    pub fn tick(&self, now: Instant) -> Vec<PendingHint> {
        let mut pending = match self.pending.lock() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        let mut expired = Vec::new();
        let mut i = 0;
        while i < pending.len() {
            if pending[i].fire_at <= now {
                expired.push(pending.swap_remove(i));
            } else {
                i += 1;
            }
        }
        expired
    }
}

impl Default for HintScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Drains expired hints from the scheduler and emits them via Tauri events.
/// Called periodically by the hint worker thread.
pub fn emit_expired_hints(app_handle: &tauri::AppHandle, scheduler: &HintScheduler) {
    let expired = scheduler.tick(Instant::now());
    for hint in expired {
        let payload = HintEvent {
            text: hint.text,
            qtype: hint.qtype.as_str().to_string(),
            session_id: hint.session_id,
        };
        if let Err(e) = app_handle.emit("new-hint", &payload) {
            eprintln!("[kue] Failed to emit expired hint: {e}");
        }
    }
}

pub fn generate_and_emit_hint(
    text: &str,
    qtype: QuestionType,
    mode: &str,
    session_id: &str,
    app_handle: Option<&tauri::AppHandle>,
    scheduler: &HintScheduler,
    db: &Database,
    model: &impl Embedder,
) {
    if qtype == QuestionType::None || text.trim().is_empty() {
        return;
    }

    let hint_text = build_hint_text(text, qtype, db, model);
    if hint_text.is_empty() {
        return;
    }

    if mode == "shadow" {
        scheduler.schedule(PendingHint {
            session_id: session_id.to_string(),
            qtype,
            text: hint_text,
            fire_at: Instant::now() + Duration::from_millis(SHADOW_DELAY_MS),
        });
    } else if let Some(handle) = app_handle {
        let payload = HintEvent {
            text: hint_text,
            qtype: qtype.as_str().to_string(),
            session_id: session_id.to_string(),
        };
        if let Err(e) = handle.emit("new-hint", &payload) {
            eprintln!("[kue] Failed to emit new-hint: {e}");
        }
    }
}

fn build_hint_text(text: &str, qtype: QuestionType, db: &Database, model: &impl Embedder) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    match search(model, db, text, HINT_TOP_K) {
        Ok(results) if !results.is_empty() => {
            let r = &results[0];
            if let (Some(tag), Some(metric)) = (&r.tag, &r.metric) {
                format_tag_metric_hint(tag, metric)
            } else {
                truncate_to_n_words(&r.text, MAX_HINT_WORDS)
            }
        }
        _ => generic_hint(qtype),
    }
}

fn format_tag_metric_hint(tag: &str, metric: &str) -> String {
    let hint = format!("💡 {}: {}", tag, metric);
    truncate_to_n_words(&hint, MAX_HINT_WORDS)
}

fn truncate_to_n_words(text: &str, n: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= n {
        text.to_string()
    } else {
        words[..n].join(" ")
    }
}

fn generic_hint(qtype: QuestionType) -> String {
    match qtype {
        QuestionType::Technical => "💡 Describe tu stack y la solución técnica".to_string(),
        QuestionType::Star => "💡 Usa STAR: Situación, Tarea, Acción, Resultado".to_string(),
        QuestionType::Architecture => "💡 Explica diseño, trade-offs y decisión final".to_string(),
        QuestionType::Trap => "💡 Sé honesto, enfócate en lo que aprendiste".to_string(),
        QuestionType::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_and_migrate;
    use crate::rag::embeddings::EMBEDDING_DIM;

    struct MockEmbedder;

    impl Embedder for MockEmbedder {
        fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
            Ok(vec![0.0f32; EMBEDDING_DIM])
        }
    }

    struct FailingEmbedder;

    impl Embedder for FailingEmbedder {
        fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
            Err("mock embedding failure".into())
        }
    }

    static VEC_INIT: std::sync::Once = std::sync::Once::new();

    fn ensure_vec_extension() {
        VEC_INIT.call_once(|| {
            crate::db::register_vec_extension();
        });
    }

    fn test_db(label: &str) -> Database {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("kue-orch-test-{label}-{id}"));
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join("kue.db");
        open_and_migrate(&path).unwrap()
    }

    fn seed_chunk(db: &Database, text: &str, tag: Option<&str>, metric: Option<&str>) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO documents (filename, type) VALUES ('test.txt', 'txt')",
            [],
        )
        .unwrap();
        let doc_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (document_id, text, chunk_index, tag, metric) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![doc_id, text, 0, tag, metric],
        )
        .unwrap();
        let chunk_id = conn.last_insert_rowid();
        let embedding: Vec<f32> = vec![0.0; EMBEDDING_DIM];
        let embedding_bytes: &[u8] = bytemuck::cast_slice(&embedding);
        conn.execute(
            "INSERT INTO chunks_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![chunk_id, embedding_bytes],
        )
        .unwrap();
        drop(conn);
    }

    // ── HintScheduler tests ──

    #[test]
    fn scheduler_tick_before_deadline_returns_nothing() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-1".into(),
            qtype: QuestionType::Technical,
            text: "not yet".into(),
            fire_at: start + Duration::from_secs(10),
        });
        let expired = s.tick(start + Duration::from_secs(5));
        assert!(expired.is_empty(), "hint should not fire before deadline");
    }

    #[test]
    fn scheduler_tick_after_deadline_returns_hint() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-2".into(),
            qtype: QuestionType::Star,
            text: "ready now".into(),
            fire_at: start + Duration::from_millis(10),
        });
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());
        assert_eq!(expired.len(), 1, "should return exactly one expired hint");
        assert_eq!(expired[0].text, "ready now");
        assert_eq!(expired[0].qtype, QuestionType::Star);
        assert_eq!(expired[0].session_id, "sess-2");
    }

    #[test]
    fn scheduler_cancel_all_removes_hints() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-cancel".into(),
            qtype: QuestionType::Trap,
            text: "should be cancelled".into(),
            fire_at: start + Duration::from_millis(10),
        });
        s.cancel_all("sess-cancel");
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());
        assert!(
            expired.is_empty(),
            "cancelled hint should never be returned by tick"
        );
    }

    #[test]
    fn scheduler_multiple_hints_with_different_sessions() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-a".into(),
            qtype: QuestionType::Technical,
            text: "hint a".into(),
            fire_at: start + Duration::from_millis(10),
        });
        s.schedule(PendingHint {
            session_id: "sess-b".into(),
            qtype: QuestionType::Star,
            text: "hint b".into(),
            fire_at: start + Duration::from_millis(10),
        });
        s.schedule(PendingHint {
            session_id: "sess-a".into(),
            qtype: QuestionType::Architecture,
            text: "hint a2".into(),
            fire_at: start + Duration::from_secs(60),
        });

        // Cancel only session "sess-a"
        s.cancel_all("sess-a");
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());

        // Only "sess-b" hint should fire
        assert_eq!(expired.len(), 1, "only sess-b hint should survive cancel_all");
        assert_eq!(expired[0].session_id, "sess-b");
        assert_eq!(expired[0].text, "hint b");

        // Second tick should return nothing (remaining hint is in the far future)
        let remaining = s.tick(Instant::now());
        assert!(remaining.is_empty());
    }

    #[test]
    fn scheduler_tick_preserves_future_hints() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-future".into(),
            qtype: QuestionType::Technical,
            text: "future hint".into(),
            fire_at: start + Duration::from_secs(60),
        });
        // Tick with a time before the hint's fire_at
        let expired = s.tick(start + Duration::from_secs(30));
        assert!(expired.is_empty(), "future hint should not expire yet");

        // The hint should still be in the queue
        let expired2 = s.tick(start + Duration::from_secs(90));
        assert_eq!(expired2.len(), 1, "future hint should expire after its deadline");
    }

    #[test]
    fn scheduler_cancel_all_unknown_session_is_noop() {
        let s = HintScheduler::new();
        let start = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-real".into(),
            qtype: QuestionType::Technical,
            text: "real hint".into(),
            fire_at: start + Duration::from_millis(10),
        });
        // Cancel a different session — should not affect our hint
        s.cancel_all("sess-other");
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());
        assert_eq!(expired.len(), 1, "cancel on unrelated session should not remove hints");
    }

    // ── build_hint_text: guard clauses ──

    #[test]
    fn no_question_returns_empty() {
        let hint =
            build_hint_text("not a question", QuestionType::None, &test_db("none"), &MockEmbedder);
        assert!(hint.is_empty());
    }

    #[test]
    fn empty_text_returns_empty() {
        let hint =
            build_hint_text("", QuestionType::Technical, &test_db("empty"), &MockEmbedder);
        assert!(hint.is_empty());
    }

    #[test]
    fn whitespace_only_text_returns_empty() {
        let hint =
            build_hint_text("   ", QuestionType::Technical, &test_db("ws"), &MockEmbedder);
        assert!(hint.is_empty());
    }

    // ── generic_hint (every variant) ──

    #[test]
    fn generic_hint_technical() {
        let h = generic_hint(QuestionType::Technical);
        assert!(h.contains("stack"));
    }

    #[test]
    fn generic_hint_star() {
        let h = generic_hint(QuestionType::Star);
        assert!(h.contains("STAR"));
    }

    #[test]
    fn generic_hint_architecture() {
        let h = generic_hint(QuestionType::Architecture);
        assert!(h.contains("diseño"));
    }

    #[test]
    fn generic_hint_trap() {
        let h = generic_hint(QuestionType::Trap);
        assert!(h.contains("honesto"));
    }

    #[test]
    fn generic_hint_none_is_empty() {
        let h = generic_hint(QuestionType::None);
        assert!(h.is_empty());
    }

    // ── truncate_to_n_words edge cases ──

    #[test]
    fn truncate_short_text_unchanged() {
        let result = truncate_to_n_words("💡 NestJS: 10k req/seg", MAX_HINT_WORDS);
        assert_eq!(result, "💡 NestJS: 10k req/seg");
    }

    #[test]
    fn truncate_long_text() {
        let long = "a b c d e f g h i j k l m n o p";
        let result = truncate_to_n_words(long, MAX_HINT_WORDS);
        assert_eq!(result, "a b c d e f g h");
        assert_eq!(result.split_whitespace().count(), MAX_HINT_WORDS);
    }

    #[test]
    fn truncate_empty_string_returns_empty() {
        assert_eq!(truncate_to_n_words("", MAX_HINT_WORDS), "");
    }

    #[test]
    fn truncate_single_word_unchanged() {
        assert_eq!(truncate_to_n_words("hello", MAX_HINT_WORDS), "hello");
    }

    #[test]
    fn truncate_exactly_n_words_unchanged() {
        let text = "one two three four five six seven eight";
        assert_eq!(truncate_to_n_words(text, MAX_HINT_WORDS), text);
    }

    #[test]
    fn truncate_zero_max_returns_empty() {
        assert_eq!(truncate_to_n_words("some words here", 0), "");
    }

    #[test]
    fn truncate_whitespace_only_unchanged() {
        assert_eq!(truncate_to_n_words("   ", MAX_HINT_WORDS), "   ");
    }

    #[test]
    fn truncate_trailing_whitespace_stripped_by_split() {
        let result = truncate_to_n_words("a b c d e f g h i j k ", MAX_HINT_WORDS);
        assert_eq!(result, "a b c d e f g h");
    }

    // ── format_tag_metric_hint edge cases ──

    #[test]
    fn format_tag_metric_hint_includes_emoji() {
        let h = format_tag_metric_hint("redis", "caché implementada");
        assert!(h.starts_with("💡"));
        assert!(h.contains("redis"));
        assert!(h.contains("caché"));
    }

    #[test]
    fn format_tag_metric_hint_truncated() {
        let h = format_tag_metric_hint("longtag", "a b c d e f g h i j k");
        assert!(h.split_whitespace().count() <= MAX_HINT_WORDS);
    }

    #[test]
    fn format_tag_metric_hint_very_short() {
        let h = format_tag_metric_hint("a", "b");
        assert_eq!(h, "💡 a: b");
    }

    #[test]
    fn format_tag_metric_hint_exact_boundary() {
        let h = format_tag_metric_hint("tag", "m1 m2 m3 m4 m5");
        assert_eq!(h.split_whitespace().count(), 7);
        assert_eq!(h, "💡 tag: m1 m2 m3 m4 m5");
    }

    // ── build_hint_text: search → empty DB → generic fallback ──

    #[test]
    fn hint_for_question_without_rag_uses_generic() {
        let model = MockEmbedder;
        let db = test_db("generic");
        let hint = build_hint_text(
            "What technology did you use?",
            QuestionType::Technical,
            &db,
            &model,
        );
        assert!(hint.contains("stack"));
    }

    #[test]
    fn hint_for_star_question_without_rag_uses_star_hint() {
        let model = MockEmbedder;
        let db = test_db("generic_star");
        let hint =
            build_hint_text("Tell me about a challenge", QuestionType::Star, &db, &model);
        assert!(hint.contains("STAR"));
    }

    // ── build_hint_text: search error → generic fallback ──

    #[test]
    fn hint_for_technical_question_with_search_error_uses_generic() {
        let db = test_db("err_tech");
        let hint = build_hint_text("anything", QuestionType::Technical, &db, &FailingEmbedder);
        assert!(hint.contains("stack"), "should fall back to generic Technical hint");
    }

    #[test]
    fn hint_for_star_question_with_search_error_uses_star_hint() {
        let db = test_db("err_star");
        let hint = build_hint_text("anything", QuestionType::Star, &db, &FailingEmbedder);
        assert!(hint.contains("STAR"), "should fall back to generic Star hint");
    }

    #[test]
    fn hint_for_architecture_question_with_search_error_uses_arch_hint() {
        let db = test_db("err_arch");
        let hint =
            build_hint_text("anything", QuestionType::Architecture, &db, &FailingEmbedder);
        assert!(hint.contains("diseño"), "should fall back to generic Architecture hint");
    }

    // ── build_hint_text: search results with tag/metric combinations ──

    #[test]
    fn hint_uses_tag_metric_when_both_present() {
        ensure_vec_extension();
        let db = test_db("both_present");
        let model = MockEmbedder;

        seed_chunk(
            &db,
            "NestJS handles 10k requests per second with Redis cache",
            Some("nestjs"),
            Some("10k req/s"),
        );

        let hint = build_hint_text("nest", QuestionType::Technical, &db, &model);
        assert!(hint.starts_with("💡"), "hint should start with 💡 emoji");
        assert!(hint.contains("nestjs"), "hint should contain tag 'nestjs'");
        assert!(hint.contains("10k"), "hint should contain metric");
    }

    #[test]
    fn hint_truncates_chunk_text_when_tag_only_present() {
        ensure_vec_extension();
        let db = test_db("tag_only");
        let model = MockEmbedder;

        let chunk_text =
            "NestJS is a progressive Node.js framework for building efficient server-side applications";
        seed_chunk(&db, chunk_text, Some("nestjs"), None);

        let hint = build_hint_text("nest", QuestionType::Technical, &db, &model);
        assert!(
            hint.contains("NestJS"),
            "hint should contain chunk text, got: {hint}"
        );
        assert!(
            !hint.starts_with("💡"),
            "hint should NOT use emoji format when metric is missing"
        );
        assert!(
            hint.split_whitespace().count() <= MAX_HINT_WORDS,
            "hint should be truncated to {MAX_HINT_WORDS} words, got {}",
            hint.split_whitespace().count()
        );
    }

    #[test]
    fn hint_truncates_chunk_text_when_metric_only_present() {
        ensure_vec_extension();
        let db = test_db("metric_only");
        let model = MockEmbedder;

        let chunk_text =
            "Achieved 40% reduction in response time through Redis caching and query optimization";
        seed_chunk(&db, chunk_text, None, Some("40% reduction"));

        let hint = build_hint_text("redis", QuestionType::Technical, &db, &model);
        assert!(
            hint.contains("response"),
            "hint should contain chunk text, got: {hint}"
        );
        assert!(
            !hint.starts_with("💡"),
            "hint should NOT use emoji format when tag is missing"
        );
        assert!(
            hint.split_whitespace().count() <= MAX_HINT_WORDS,
            "hint should be truncated to {MAX_HINT_WORDS} words"
        );
    }

    #[test]
    fn hint_truncates_chunk_text_when_neither_tag_nor_metric_present() {
        ensure_vec_extension();
        let db = test_db("no_tag_metric");
        let model = MockEmbedder;

        let chunk_text =
            "Rust guarantees memory safety without a garbage collector through its ownership system";
        seed_chunk(&db, chunk_text, None, None);

        let hint = build_hint_text("rust", QuestionType::Technical, &db, &model);
        assert!(
            hint.contains("Rust"),
            "hint should contain chunk text, got: {hint}"
        );
        assert!(
            hint.split_whitespace().count() <= MAX_HINT_WORDS,
            "hint should be truncated to {MAX_HINT_WORDS} words, got {}",
            hint.split_whitespace().count()
        );
    }

    // ── HintEvent serialization ──

    #[test]
    fn hint_event_serializes_with_renamed_type_field() {
        let event = HintEvent {
            text: "💡 redis: caché implementada".to_string(),
            qtype: "technical".to_string(),
            session_id: "session-456".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("type").is_some(),
            "should have a 'type' field (renamed from qtype), got: {json}"
        );
        assert!(
            parsed.get("qtype").is_none(),
            "should NOT have a 'qtype' field (renamed to 'type'), got: {json}"
        );
        assert_eq!(parsed["type"], "technical");
        assert_eq!(parsed["text"], "💡 redis: caché implementada");
        assert_eq!(parsed["session_id"], "session-456");
    }

    // ── Integration: full generate_and_emit_hint → scheduler → cancel path ──

    /// Helpers for the integration tests: calls generate_and_emit_hint in
    /// shadow mode and returns the scheduler so the caller can verify state.
    fn schedule_shadow_hint(
        scheduler: &HintScheduler,
        session_id: &str,
        text: &str,
        qtype: QuestionType,
        db: &Database,
        model: &MockEmbedder,
    ) {
        generate_and_emit_hint(
            text,
            qtype,
            "shadow",
            session_id,
            None, // no AppHandle needed in shadow mode
            scheduler,
            db,
            model,
        );
    }

    #[test]
    fn cancel_session_prevents_shadow_hint_from_firing() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("cancel_integration");
        let scheduler = HintScheduler::new();
        let session_id = "sess-cancel-int";

        // Schedule a shadow hint (simulates what the worker does after
        // receiving HintCommand::Process in shadow mode).
        schedule_shadow_hint(&scheduler, session_id, "What is Rust?", QuestionType::Technical, &db, &model);

        // Simulate session end — same CancelSession command that the
        // pipeline sends when its thread exits.
        scheduler.cancel_all(session_id);

        // Advance simulated time well past the 2.5s delay.
        // The hint's fire_at was set to now+2.5s at scheduling time;
        // by sending a now+5s we guarantee we're past the deadline.
        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert!(
            expired.is_empty(),
            "CancelSession should prevent the shadow hint from being returned by tick(), got {} hints",
            expired.len()
        );
    }

    #[test]
    fn without_cancel_shadow_hint_fires_after_delay() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("nocancel_integration");
        let scheduler = HintScheduler::new();
        let session_id = "sess-nocancel";
        let text = "Tell me about a conflict";

        schedule_shadow_hint(&scheduler, session_id, text, QuestionType::Star, &db, &model);

        // Advance simulated time well past the 2.5s delay.
        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(expired.len(), 1, "without CancelSession the hint should fire");
        assert_eq!(expired[0].session_id, session_id);
        // The hint text is built from RAG (generic Star hint since DB is empty)
        assert!(
            expired[0].text.contains("STAR"),
            "hint text should contain the generic Star hint, got: {}",
            expired[0].text
        );
        assert_eq!(expired[0].qtype, QuestionType::Star, "qtype should be preserved through the roundtrip");

        // Second tick should return nothing (hint was consumed)
        let again = scheduler.tick(future);
        assert!(again.is_empty(), "hint should not appear twice");
    }

    /// Combined test: schedule two hints for different sessions, cancel one,
    /// verify only the other fires. Reproduces a real interview scenario where
    /// one session ends while another is still active.
    #[test]
    fn cancel_session_does_not_affect_other_sessions() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("multi_session");
        let scheduler = HintScheduler::new();

        schedule_shadow_hint(&scheduler, "sess-a", "question a", QuestionType::Technical, &db, &model);
        schedule_shadow_hint(&scheduler, "sess-b", "question b", QuestionType::Star, &db, &model);

        // End session "sess-a"
        scheduler.cancel_all("sess-a");

        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(expired.len(), 1, "only sess-b's hint should survive");
        assert_eq!(expired[0].session_id, "sess-b");
        assert!(expired[0].text.contains("STAR"), "should be the Star generic hint for sess-b");
    }
}
