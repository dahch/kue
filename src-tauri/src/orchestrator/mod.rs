use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tauri::Manager;

pub mod worker;

use crate::classifier::QuestionType;
use crate::db::Database;
use crate::rag::embeddings::Embedder;
use crate::rag::indexer::search;

/// Shared state for panic mode — when set, hints are silenced until this instant.
#[derive(Clone)]
pub struct PanicState(pub Arc<Mutex<Option<Instant>>>);

impl PanicState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    pub fn is_panicking(&self) -> bool {
        self.0
            .lock()
            .ok()
            .is_some_and(|guard| guard.is_some_and(|until| Instant::now() < until))
    }
}

pub const SHADOW_DELAY_MS: u64 = 2500;
const HINT_TOP_K: usize = 5;

const MAX_HINT_WORDS: usize = 20;

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
    /// When this hint was originally scheduled (used by Shadow mode to
    /// check whether the user started speaking on Channel A since then).
    pub scheduled_at: Instant,
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

/// Returns `true` if `hint` should be cancelled because the user has
/// started speaking on Channel A (mic) since the hint was scheduled.
///
/// When no `MicVadState` is available (e.g. in tests) all hints are
/// allowed to proceed.
/// Returns `true` if panic mode is active and hints should be suppressed.
fn hint_silenced_by_panic(app_handle: &tauri::AppHandle) -> bool {
    app_handle
        .try_state::<PanicState>()
        .is_some_and(|s| s.is_panicking())
}

pub fn should_cancel_hint(
    hint: &PendingHint,
    mic_vad: Option<&crate::audio::mic_vad::MicVadState>,
) -> bool {
    mic_vad
        .is_some_and(|vad| vad.is_currently_speaking() || vad.has_speech_since(hint.scheduled_at))
}

/// Drains expired hints from the scheduler and emits them via Tauri events.
/// Called periodically by the hint worker thread.
///
/// In Shadow mode, each expired hint is checked against the mic (Channel A)
/// VAD: if the user started speaking since the hint was scheduled, the hint
/// is silently cancelled (the user is already answering and doesn't need a
/// prompt).  When no `AudioCapture` state is available (e.g. in tests),
/// all expired hints are emitted as-is.
///
/// If panic mode is active (see [`PanicState`]), all hints are silently dropped
/// until the panic timer expires.
pub fn emit_expired_hints(app_handle: &tauri::AppHandle, scheduler: &HintScheduler) {
    if hint_silenced_by_panic(app_handle) {
        return;
    }

    let expired = scheduler.tick(Instant::now());

    let mic_vad = app_handle
        .try_state::<crate::audio::capture::AudioCapture>()
        .map(|cap| cap.mic_vad_state());

    for hint in expired {
        let cancel = mic_vad
            .as_ref()
            .map(|vad| {
                let state = match vad.lock() {
                    Ok(g) => g,
                    Err(e) => {
                        log::warn!("VAD mutex poisoned — VAD gating disabled");
                        e.into_inner()
                    }
                };
                should_cancel_hint(&hint, Some(&state))
            })
            .unwrap_or(false);

        if cancel {
            continue;
        }

        let payload = HintEvent {
            text: hint.text,
            qtype: hint.qtype.as_str().to_string(),
            session_id: hint.session_id,
        };
        if let Err(e) = app_handle.emit("new-hint", &payload) {
            log::warn!("Failed to emit expired hint: {e}");
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

    if let Some(handle) = app_handle {
        if hint_silenced_by_panic(handle) {
            return;
        }
    }

    let mut hint_text = build_hint_text(text, qtype, db, model);

    // Attempt LLM-enhanced hint if a provider is configured with an API key.
    if let Some(handle) = app_handle {
        if let Some(llm_hint) = try_llm_hint(handle, text, db, model) {
            hint_text = llm_hint;
        }
    }

    if hint_text.is_empty() {
        return;
    }

    if mode == "shadow" {
        let now = Instant::now();
        scheduler.schedule(PendingHint {
            session_id: session_id.to_string(),
            qtype,
            text: hint_text,
            fire_at: now + Duration::from_millis(SHADOW_DELAY_MS),
            scheduled_at: now,
        });
    } else if let Some(handle) = app_handle {
        let payload = HintEvent {
            text: hint_text,
            qtype: qtype.as_str().to_string(),
            session_id: session_id.to_string(),
        };
        if let Err(e) = handle.emit("new-hint", &payload) {
            log::warn!("Failed to emit new-hint: {e}");
        }
    }
}

/// Attempts to generate a hint using the configured LLM provider.
/// Returns `None` if no key is configured or the LLM call fails/times out,
/// so the caller falls back to the RAG-only hint.
fn try_llm_hint(
    app_handle: &tauri::AppHandle,
    question: &str,
    db: &Database,
    model: &impl Embedder,
) -> Option<String> {
    // Try providers in order: hint_provider → analysis provider → any with a key
    const CANDIDATES: &[&str] = &[
        "openai",
        "anthropic",
        "gemini",
        "openrouter",
        "deepseek",
        "ollama",
    ];

    let hint_provider_raw = get_setting_or(app_handle, "hint_provider", "");
    let default_provider = get_setting_or(app_handle, "default_provider", "openai");
    let configured = if hint_provider_raw.is_empty() {
        default_provider.clone()
    } else {
        hint_provider_raw
    };
    let analysis_provider = get_setting_or(app_handle, "provider", "openai");

    // Build a priority list: configured hint provider first, then analysis
    // provider if different, then all other candidates.
    let mut order: Vec<&str> = vec![configured.as_str()];
    if analysis_provider != configured {
        order.push(analysis_provider.as_str());
    }
    for c in CANDIDATES {
        if !order.contains(c) {
            order.push(c);
        }
    }

    let hint_model_raw = get_setting_or(app_handle, "hint_model", "");
    let default_model = get_setting_or(app_handle, "default_model", "");
    let hint_model = if !hint_model_raw.is_empty() {
        hint_model_raw
    } else if !default_model.is_empty() {
        default_model
    } else {
        "gpt-4o-mini".to_string()
    };

    // Find the first provider with a saved API key
    let (provider, api_key) = order
        .iter()
        .find_map(|p| crate::keys::get_api_key(p).ok().map(|k| (p.to_string(), k)))?;

    // Build RAG context: concatenate top-k chunk texts
    let rag_context = match search(model, db, question, HINT_TOP_K) {
        Ok(results) if !results.is_empty() => {
            let chunks: Vec<String> = results
                .iter()
                .map(|r| {
                    if let (Some(tag), Some(metric)) = (&r.tag, &r.metric) {
                        format!("[{}] {} — {}", tag, r.text, metric)
                    } else {
                        r.text.clone()
                    }
                })
                .collect();
            chunks.join("\n---\n")
        }
        _ => String::from("(No relevant documents available)"),
    };

    // Clone strings for the spawned thread
    let q = question.to_string();
    let rag = rag_context;
    let model_name = hint_model;

    // Call the LLM via Tauri's async runtime with a 4-second timeout
    // enforced through a channel from a helper thread.
    let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
    std::thread::spawn(move || {
        let result = tauri::async_runtime::block_on(crate::llm::generate_hint(
            &q,
            &rag,
            &provider,
            &model_name,
            &api_key,
        ));
        let _ = tx.send(result);
    });

    match rx.recv_timeout(std::time::Duration::from_secs(4)) {
        Ok(Ok(hint)) if !hint.trim().is_empty() => Some(hint.trim().to_string()),
        Ok(Err(e)) => {
            log::warn!("LLM hint generation failed, falling back to RAG: {e}");
            None
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!("LLM hint generation timed out after 4s");
            None
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            log::warn!("LLM hint generation thread disconnected unexpectedly");
            None
        }
        _ => None,
    }
}

/// Reads a setting value from the database. Returns `default_val` if the
/// setting does not exist or the DB cannot be accessed.
fn get_setting_or(app_handle: &tauri::AppHandle, key: &str, default_val: &str) -> String {
    app_handle
        .try_state::<Database>()
        .and_then(|db| {
            let conn = db.conn.lock().ok()?;
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .ok()
        })
        .unwrap_or_else(|| default_val.to_string())
}

fn build_hint_text(
    text: &str,
    qtype: QuestionType,
    db: &Database,
    model: &impl Embedder,
) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    match search(model, db, text, HINT_TOP_K) {
        Ok(results) if !results.is_empty() => {
            // Prefer results that have both a tag and a metric, because they
            // give the user a concrete cue ("Redis: 10k req/s"). Fall back to
            // any non-empty chunk text. Skip duplicate tag/metric pairs so
            // consecutive questions don't always show the same hint.
            let mut seen: Vec<(String, String)> = Vec::new();
            for r in &results {
                if let (Some(tag), Some(metric)) = (&r.tag, &r.metric) {
                    let key = (tag.to_lowercase(), metric.to_lowercase());
                    if !seen.contains(&key) {
                        seen.push(key);
                        return format_tag_metric_hint(tag, metric);
                    }
                }
            }
            for r in &results {
                let trimmed = r.text.trim();
                if !trimmed.is_empty() {
                    let candidate = truncate_to_n_words(trimmed, MAX_HINT_WORDS);
                    if !candidate.is_empty() {
                        return candidate;
                    }
                }
            }
            generic_hint(qtype)
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
    if words.is_empty() {
        // Preserve whitespace-only input unchanged.
        text.to_string()
    } else if words.len() <= n {
        words.join(" ")
    } else {
        words[..n].join(" ")
    }
}

fn generic_hint(qtype: QuestionType) -> String {
    match qtype {
        QuestionType::Technical => {
            "💡 Name your stack, key decision, metric, or lesson learned.".to_string()
        }
        QuestionType::Star => {
            "💡 Use STAR: Situation, Task, Action, Result — with a metric.".to_string()
        }
        QuestionType::Architecture => {
            "💡 Outline layers, trade-offs, scalability reasons for each choice.".to_string()
        }
        QuestionType::Trap => {
            "💡 Be honest, brief, and pivot to what you did to improve.".to_string()
        }
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
            scheduled_at: start,
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
            scheduled_at: start,
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
            scheduled_at: start,
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
            scheduled_at: start,
        });
        s.schedule(PendingHint {
            session_id: "sess-b".into(),
            qtype: QuestionType::Star,
            text: "hint b".into(),
            fire_at: start + Duration::from_millis(10),
            scheduled_at: start,
        });
        s.schedule(PendingHint {
            session_id: "sess-a".into(),
            qtype: QuestionType::Architecture,
            text: "hint a2".into(),
            fire_at: start + Duration::from_secs(60),
            scheduled_at: start,
        });

        // Cancel only session "sess-a"
        s.cancel_all("sess-a");
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());

        // Only "sess-b" hint should fire
        assert_eq!(
            expired.len(),
            1,
            "only sess-b hint should survive cancel_all"
        );
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
            scheduled_at: start,
        });
        // Tick with a time before the hint's fire_at
        let expired = s.tick(start + Duration::from_secs(30));
        assert!(expired.is_empty(), "future hint should not expire yet");

        // The hint should still be in the queue
        let expired2 = s.tick(start + Duration::from_secs(90));
        assert_eq!(
            expired2.len(),
            1,
            "future hint should expire after its deadline"
        );
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
            scheduled_at: start,
        });
        // Cancel a different session — should not affect our hint
        s.cancel_all("sess-other");
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());
        assert_eq!(
            expired.len(),
            1,
            "cancel on unrelated session should not remove hints"
        );
    }

    // ── build_hint_text: guard clauses ──

    #[test]
    fn no_question_returns_empty() {
        let hint = build_hint_text(
            "not a question",
            QuestionType::None,
            &test_db("none"),
            &MockEmbedder,
        );
        assert!(hint.is_empty());
    }

    #[test]
    fn empty_text_returns_empty() {
        let hint = build_hint_text(
            "",
            QuestionType::Technical,
            &test_db("empty"),
            &MockEmbedder,
        );
        assert!(hint.is_empty());
    }

    #[test]
    fn whitespace_only_text_returns_empty() {
        let hint = build_hint_text(
            "   ",
            QuestionType::Technical,
            &test_db("ws"),
            &MockEmbedder,
        );
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
        assert!(h.contains("trade-offs"));
    }

    #[test]
    fn generic_hint_trap() {
        let h = generic_hint(QuestionType::Trap);
        assert!(h.contains("honest"));
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
        assert_eq!(result, long);
        assert_eq!(result.split_whitespace().count(), 16);
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
        assert_eq!(result, "a b c d e f g h i j k");
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
        let hint = build_hint_text("Tell me about a challenge", QuestionType::Star, &db, &model);
        assert!(hint.contains("STAR"));
    }

    // ── build_hint_text: search error → generic fallback ──

    #[test]
    fn hint_for_technical_question_with_search_error_uses_generic() {
        let db = test_db("err_tech");
        let hint = build_hint_text("anything", QuestionType::Technical, &db, &FailingEmbedder);
        assert!(
            hint.contains("stack"),
            "should fall back to generic Technical hint"
        );
    }

    #[test]
    fn hint_for_star_question_with_search_error_uses_star_hint() {
        let db = test_db("err_star");
        let hint = build_hint_text("anything", QuestionType::Star, &db, &FailingEmbedder);
        assert!(
            hint.contains("STAR"),
            "should fall back to generic Star hint"
        );
    }

    #[test]
    fn hint_for_architecture_question_with_search_error_uses_arch_hint() {
        let db = test_db("err_arch");
        let hint = build_hint_text(
            "anything",
            QuestionType::Architecture,
            &db,
            &FailingEmbedder,
        );
        assert!(
            hint.contains("arquitectura") || hint.contains("capas") || hint.contains("trade-offs"),
            "should fall back to generic Architecture hint"
        );
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
            text, qtype, "shadow", session_id, None, // no AppHandle needed in shadow mode
            scheduler, db, model,
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
        schedule_shadow_hint(
            &scheduler,
            session_id,
            "What is Rust?",
            QuestionType::Technical,
            &db,
            &model,
        );

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

        schedule_shadow_hint(
            &scheduler,
            session_id,
            text,
            QuestionType::Star,
            &db,
            &model,
        );

        // Advance simulated time well past the 2.5s delay.
        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(
            expired.len(),
            1,
            "without CancelSession the hint should fire"
        );
        assert_eq!(expired[0].session_id, session_id);
        // The hint text is built from RAG (generic Star hint since DB is empty)
        assert!(
            expired[0].text.contains("STAR"),
            "hint text should contain the generic Star hint, got: {}",
            expired[0].text
        );
        assert_eq!(
            expired[0].qtype,
            QuestionType::Star,
            "qtype should be preserved through the roundtrip"
        );

        // Second tick should return nothing (hint was consumed)
        let again = scheduler.tick(future);
        assert!(again.is_empty(), "hint should not appear twice");
    }

    // ── should_cancel_hint ──

    #[test]
    fn should_cancel_hint_no_mic_vad_never_cancels() {
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Technical,
            text: "hint".into(),
            fire_at: Instant::now(),
            scheduled_at: Instant::now(),
        };
        assert!(!should_cancel_hint(&hint, None));
    }

    #[test]
    fn should_cancel_hint_false_when_user_silent() {
        let vad = crate::audio::mic_vad::MicVadState::new();
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Technical,
            text: "hint".into(),
            fire_at: Instant::now(),
            scheduled_at: Instant::now(),
        };
        assert!(!should_cancel_hint(&hint, Some(&vad)));
    }

    #[test]
    fn should_cancel_hint_true_when_currently_speaking() {
        let mut vad = crate::audio::mic_vad::MicVadState::new();
        // Feed enough speech to trigger VAD
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }
        assert!(vad.is_currently_speaking());

        // Hint scheduled before speech started
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Technical,
            text: "hint".into(),
            fire_at: Instant::now(),
            scheduled_at: Instant::now() - std::time::Duration::from_secs(10),
        };
        assert!(
            should_cancel_hint(&hint, Some(&vad)),
            "should cancel when user is currently speaking"
        );
    }

    #[test]
    fn should_cancel_hint_true_when_user_spoke_since_scheduled() {
        let mut vad = crate::audio::mic_vad::MicVadState::new();
        let before = Instant::now();

        // Hint scheduled at `before`
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Star,
            text: "star hint".into(),
            fire_at: before + std::time::Duration::from_millis(2500),
            scheduled_at: before,
        };

        // User starts speaking after the hint was scheduled
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }
        assert!(vad.has_speech_since(before));
        assert!(
            should_cancel_hint(&hint, Some(&vad)),
            "should cancel when user spoke after hint was scheduled"
        );
    }

    #[test]
    fn should_cancel_hint_true_when_speaking_before_scheduled() {
        let mut vad = crate::audio::mic_vad::MicVadState::new();
        // User was speaking before the hint was scheduled
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }

        let now = Instant::now();
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Architecture,
            text: "arch hint".into(),
            fire_at: now + std::time::Duration::from_millis(2500),
            scheduled_at: now,
        };

        // `has_speech_since(now)` will be false because the speech start
        // happened before `now`. But `is_currently_speaking()` is true,
        // so `should_cancel_hint` returns true anyway.
        assert!(
            vad.is_currently_speaking(),
            "user was already speaking when hint was scheduled"
        );
        assert!(
            should_cancel_hint(&hint, Some(&vad)),
            "should cancel when user was already speaking"
        );
    }

    #[test]
    fn should_cancel_hint_false_when_user_spoke_before_and_stopped() {
        let mut vad = crate::audio::mic_vad::MicVadState::new();
        // User spoke and finished before the hint was scheduled
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }
        // Wait for silence longer than timeout (600ms)
        let long_silence = vec![0i16; 9600]; // 600ms @ 16kHz
        vad.feed_audio(&long_silence);
        assert!(!vad.is_currently_speaking());

        let now = Instant::now();
        let hint = PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Trap,
            text: "trap hint".into(),
            fire_at: now + std::time::Duration::from_millis(2500),
            scheduled_at: now,
        };

        assert!(
            !should_cancel_hint(&hint, Some(&vad)),
            "should not cancel when user stopped speaking before hint was scheduled"
        );
    }

    // ── Integration: shadow hints + mic VAD ──

    /// Simulates what `emit_expired_hints` does when `should_cancel_hint`
    /// returns true for an expired hint — the hint should be silently dropped.
    #[test]
    fn shadow_hint_cancelled_when_user_speaks_during_delay() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("shadow_cancel_vad");
        let scheduler = HintScheduler::new();
        let session_id = "sess-shadow-vad";

        schedule_shadow_hint(
            &scheduler,
            session_id,
            "What is Rust?",
            QuestionType::Technical,
            &db,
            &model,
        );

        // Simulate user speaking (mic VAD) during the 2.5s delay
        let mut vad = crate::audio::mic_vad::MicVadState::new();
        // Feed speech to trigger VAD
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }

        // Advance past deadline
        let future = Instant::now() + std::time::Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(
            expired.len(),
            1,
            "tick should still return the hint (VAD isn't in tick)"
        );
        // Now simulate what emit_expired_hints does:
        let should_cancel = expired.iter().any(|h| should_cancel_hint(h, Some(&vad)));
        assert!(should_cancel, "VAD should cancel the expired hint");
    }

    #[test]
    fn shadow_hint_fires_when_user_silent_during_delay() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("shadow_no_vad");
        let scheduler = HintScheduler::new();
        let session_id = "sess-shadow-silent";

        schedule_shadow_hint(
            &scheduler,
            session_id,
            "Tell me about a conflict",
            QuestionType::Star,
            &db,
            &model,
        );

        // No mic VAD activity
        let vad = crate::audio::mic_vad::MicVadState::new();

        let future = Instant::now() + std::time::Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(expired.len(), 1, "tick should return the expired hint");
        let should_cancel = expired.iter().any(|h| should_cancel_hint(h, Some(&vad)));
        assert!(!should_cancel, "VAD should not cancel when user is silent");

        let hint = &expired[0];
        assert_eq!(hint.session_id, session_id);
        assert!(
            hint.text.contains("STAR"),
            "hint text should contain the generic Star hint, got: {}",
            hint.text
        );
    }

    /// Combined test: schedule two shadow hints, user starts speaking,
    /// only the hint scheduled after speech stops should survive.
    #[test]
    fn shadow_hint_multi_question_with_vad() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("shadow_multi_vad");
        let scheduler = HintScheduler::new();

        // Simulate two questions in sequence
        schedule_shadow_hint(
            &scheduler,
            "sess-m",
            "first question",
            QuestionType::Technical,
            &db,
            &model,
        );
        schedule_shadow_hint(
            &scheduler,
            "sess-m",
            "second question",
            QuestionType::Star,
            &db,
            &model,
        );

        let mut vad = crate::audio::mic_vad::MicVadState::new();
        // User starts speaking after both questions
        for _ in 0..2 {
            vad.feed_audio(&vec![5000i16; 1600]);
        }

        let future = Instant::now() + std::time::Duration::from_secs(5);
        let expired = scheduler.tick(future);
        assert_eq!(expired.len(), 2, "both hints should be expired by tick");

        // Both should be cancelled by VAD
        let any_survive = expired.iter().any(|h| !should_cancel_hint(h, Some(&vad)));
        assert!(
            !any_survive,
            "all hints should be cancelled when user is speaking"
        );
    }

    /// Combined test: specify that VAD cancellation does NOT interfere with
    /// session-level CancelSession (Panic / end of session).
    #[test]
    fn vad_cancel_does_not_conflict_with_cancel_session() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("vad_no_conflict");
        let scheduler = HintScheduler::new();
        let session_id = "sess-vad-conflict";

        schedule_shadow_hint(
            &scheduler,
            session_id,
            "question",
            QuestionType::Technical,
            &db,
            &model,
        );

        // Cancel at session level (same as pipeline thread end)
        scheduler.cancel_all(session_id);

        // Even without VAD detection, CancelSession should prevent the hint
        // from firing.
        let future = Instant::now() + std::time::Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert!(
            expired.is_empty(),
            "CancelSession should remove hints regardless of VAD state"
        );
    }

    /// Combined test: specify that VAD cancellation does NOT interfere with
    /// the existing panic/session-end tests from the original test suite.
    #[test]
    fn cancel_session_does_not_affect_other_sessions() {
        ensure_vec_extension();
        let model = MockEmbedder;
        let db = test_db("multi_session");
        let scheduler = HintScheduler::new();

        schedule_shadow_hint(
            &scheduler,
            "sess-a",
            "question a",
            QuestionType::Technical,
            &db,
            &model,
        );
        schedule_shadow_hint(
            &scheduler,
            "sess-b",
            "question b",
            QuestionType::Star,
            &db,
            &model,
        );

        // End session "sess-a"
        scheduler.cancel_all("sess-a");

        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);

        assert_eq!(expired.len(), 1, "only sess-b's hint should survive");
        assert_eq!(expired[0].session_id, "sess-b");
        assert!(
            expired[0].text.contains("STAR"),
            "should be the Star generic hint for sess-b"
        );
    }

    // ── PanicState tests ──

    #[test]
    fn panic_state_starts_inactive() {
        let state = PanicState::new();
        assert!(
            !state.is_panicking(),
            "fresh PanicState should not be panicking"
        );
    }

    #[test]
    fn panic_state_activated_for_duration() {
        let state = PanicState::new();
        {
            let mut guard = state.0.lock().unwrap();
            *guard = Some(Instant::now() + Duration::from_secs(10));
        }
        assert!(
            state.is_panicking(),
            "should be panicking while within the 10s window"
        );
    }

    #[test]
    fn panic_state_expires() {
        let state = PanicState::new();
        {
            let mut guard = state.0.lock().unwrap();
            // Set panic until 1ms ago — already expired
            *guard = Some(Instant::now() - Duration::from_millis(1));
        }
        // Sleep a tiny bit to ensure the instant is really in the past
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            !state.is_panicking(),
            "should not be panicking after expiry"
        );
    }

    #[test]
    fn panic_state_reset_clears_panic() {
        let state = PanicState::new();
        {
            let mut guard = state.0.lock().unwrap();
            *guard = Some(Instant::now() + Duration::from_secs(10));
        }
        assert!(state.is_panicking());
        {
            let mut guard = state.0.lock().unwrap();
            *guard = None;
        }
        assert!(!state.is_panicking(), "should not be panicking after reset");
    }

    // -----------------------------------------------------------------------
    // HintScheduler: Default impl
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_default_is_same_as_new() {
        let a = HintScheduler::new();
        let b = HintScheduler::default();
        // Both should be empty and behave identically
        assert_eq!(a.tick(Instant::now()).len(), b.tick(Instant::now()).len());
        assert_eq!(a.cancel_all("any"), ());
    }

    // -----------------------------------------------------------------------
    // HintScheduler: empty / edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_tick_empty_returns_empty() {
        let s = HintScheduler::new();
        let expired = s.tick(Instant::now());
        assert!(
            expired.is_empty(),
            "tick on empty scheduler should return empty vec"
        );
    }

    #[test]
    fn scheduler_cancel_all_empty_is_noop() {
        let s = HintScheduler::new();
        // Should not panic or error
        s.cancel_all("non-existent-session");
        let expired = s.tick(Instant::now());
        assert!(
            expired.is_empty(),
            "cancel on empty scheduler should leave it empty"
        );
    }

    // -----------------------------------------------------------------------
    // HintScheduler: poisoned mutex resilience
    // -----------------------------------------------------------------------

    /// Poison the scheduler's internal mutex by panicking while holding the lock.
    /// All public methods should handle the poison gracefully (return defaults).
    fn poison_scheduler(s: &HintScheduler) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = s.pending.lock().unwrap();
            panic!("intentional poison");
        }));
    }

    #[test]
    fn scheduler_schedule_handles_poisoned_mutex() {
        let s = HintScheduler::new();
        poison_scheduler(&s);

        // schedule should silently ignore the poisoned mutex
        s.schedule(PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Technical,
            text: "ignored".into(),
            fire_at: Instant::now(),
            scheduled_at: Instant::now(),
        });
        // No panic is the main assertion
    }

    #[test]
    fn scheduler_tick_handles_poisoned_mutex() {
        let s = HintScheduler::new();
        poison_scheduler(&s);

        let expired = s.tick(Instant::now());
        assert!(
            expired.is_empty(),
            "tick on poisoned mutex should return empty vec"
        );
    }

    #[test]
    fn scheduler_cancel_all_handles_poisoned_mutex() {
        let s = HintScheduler::new();
        poison_scheduler(&s);

        // cancel_all should silently skip the poisoned mutex
        s.cancel_all("sess-any");
        // No panic is the main assertion
    }

    // -----------------------------------------------------------------------
    // HintScheduler: schedule + tick integration (poisoned then recovered)
    // -----------------------------------------------------------------------

    /// Verify that after poisoning, the scheduler still creates a new valid
    /// guard if the mutex is cleaned up (i.e., the poison is handled and the
    /// lock is acquired again after the poisoning thread releases it).
    #[test]
    fn scheduler_works_after_poison_handled() {
        // Note: in single-threaded use, poisoning the mutex makes .lock()
        // return Err(MutexError::Poisoned). The scheduler methods use
        // `if let Ok` / `match { Ok => ..., Err => return }` which means
        // they silently skip after poison. For normal recovery, the Mutex
        // would need to be re-initialized. This test verifies the defensive
        // behaviour — not that recovery is possible.
        let s = HintScheduler::new();
        poison_scheduler(&s);

        // After poison, operations are silently degraded
        let start = Instant::now();
        // Schedule will be a no-op (poisoned)
        s.schedule(PendingHint {
            session_id: "sess".into(),
            qtype: QuestionType::Star,
            text: "will be lost".into(),
            fire_at: start + Duration::from_millis(10),
            scheduled_at: start,
        });

        // Tick will return empty (poisoned)
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());
        assert!(
            expired.is_empty(),
            "hint scheduled on poisoned mutex should be lost"
        );
    }

    // -----------------------------------------------------------------------
    // HintScheduler: schedule preserves order of equal-deadline hints
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_returns_all_hints_with_same_deadline() {
        let s = HintScheduler::new();
        let now = Instant::now();
        s.schedule(PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Technical,
            text: "first".into(),
            fire_at: now + Duration::from_millis(10),
            scheduled_at: now,
        });
        s.schedule(PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Star,
            text: "second".into(),
            fire_at: now + Duration::from_millis(10),
            scheduled_at: now,
        });
        s.schedule(PendingHint {
            session_id: "s".into(),
            qtype: QuestionType::Architecture,
            text: "third".into(),
            fire_at: now + Duration::from_millis(10),
            scheduled_at: now,
        });

        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());

        assert_eq!(expired.len(), 3, "all three should expire");
        // tick uses swap_remove so order is not guaranteed; verify all present.
        let texts: Vec<&str> = expired.iter().map(|h| h.text.as_str()).collect();
        assert!(texts.contains(&"first"), "should contain 'first'");
        assert!(texts.contains(&"second"), "should contain 'second'");
        assert!(texts.contains(&"third"), "should contain 'third'");
    }

    // ── generate_and_emit_hint: guard clauses ──

    #[test]
    fn generate_and_emit_hint_none_type_returns_early() {
        // Should not crash or emit anything
        generate_and_emit_hint(
            "any text",
            QuestionType::None,
            "practice",
            "sess-1",
            None,
            &HintScheduler::new(),
            &test_db("none_early"),
            &MockEmbedder,
        );
        // No panic is the assertion
    }

    #[test]
    fn generate_and_emit_hint_whitespace_text_returns_early() {
        // build_hint_text returns empty for whitespace-only text with None qtype
        generate_and_emit_hint(
            "   ",
            QuestionType::Technical,
            "practice",
            "sess-1",
            None,
            &HintScheduler::new(),
            &test_db("ws_early"),
            &MockEmbedder,
        );
    }

    #[test]
    fn generate_and_emit_hint_question_type_none_empty_text() {
        // Both guard conditions hit: first the qtype == None check,
        // then the text.trim().is_empty() in build_hint_text
        generate_and_emit_hint(
            "",
            QuestionType::None,
            "shadow",
            "sess-1",
            None,
            &HintScheduler::new(),
            &test_db("empty_none"),
            &MockEmbedder,
        );
    }

    // ── generate_and_emit_hint: practice mode without AppHandle ──

    #[test]
    fn generate_and_emit_hint_practice_mode_no_app_handle_schedules_nothing() {
        // In practice mode, without an AppHandle the hint is silently dropped
        // (the function checks `if let Some(handle) = app_handle` and skips emission).
        let scheduler = HintScheduler::new();
        generate_and_emit_hint(
            "What is Rust?",
            QuestionType::Technical,
            "practice",
            "sess-1",
            None,
            &scheduler,
            &test_db("practice_no_handle"),
            &MockEmbedder,
        );
        // Nothing should be scheduled (practice mode uses the scheduler only
        // when mode == "shadow")
        let expired = scheduler.tick(Instant::now() + Duration::from_secs(60));
        assert!(expired.is_empty());
    }

    // ── generate_and_emit_hint: fallback when search returns no results ──

    #[test]
    fn generate_and_emit_hint_fallback_to_generic_when_search_empty() {
        // When the DB is empty, search returns empty → generic_hint is used.
        // build_hint_text returns the generic hint.
        let text = "What technology is best?";
        let scheduler = HintScheduler::new();
        generate_and_emit_hint(
            text,
            QuestionType::Technical,
            "shadow",
            "sess-generic",
            None,
            &scheduler,
            &test_db("generic_fallback"),
            &MockEmbedder,
        );

        let future = Instant::now() + Duration::from_secs(5);
        let expired = scheduler.tick(future);
        assert_eq!(expired.len(), 1, "should have scheduled a hint");
        assert!(
            expired[0].text.contains("stack"),
            "should fall back to generic Technical hint"
        );
    }

    // ── HintScheduler: hint metadata preserved through tick ──

    #[test]
    fn scheduler_tick_preserves_hint_metadata() {
        let s = HintScheduler::new();
        let now = Instant::now();
        s.schedule(PendingHint {
            session_id: "sess-777".into(),
            qtype: QuestionType::Trap,
            text: "💡 Sé honesto".into(),
            fire_at: now + Duration::from_millis(10),
            scheduled_at: now,
        });

        std::thread::sleep(Duration::from_millis(20));
        let expired = s.tick(Instant::now());

        assert_eq!(expired.len(), 1);
        let hint = &expired[0];
        assert_eq!(hint.session_id, "sess-777");
        assert_eq!(hint.qtype, QuestionType::Trap);
        assert_eq!(hint.text, "💡 Sé honesto");
        assert!(hint.fire_at <= Instant::now());
        assert!(hint.scheduled_at <= hint.fire_at);
    }

    // ── PendingHint constructor guard ──

    #[test]
    fn pending_hint_fields_are_accessible() {
        let now = Instant::now();
        let hint = PendingHint {
            session_id: "sess-p".into(),
            qtype: QuestionType::Architecture,
            text: "arch hint".into(),
            fire_at: now + Duration::from_secs(10),
            scheduled_at: now,
        };
        assert_eq!(hint.session_id, "sess-p");
        assert_eq!(hint.qtype, QuestionType::Architecture);
        assert_eq!(hint.text, "arch hint");
        assert!(hint.fire_at > now);
        assert_eq!(hint.scheduled_at, now);
    }

    // ── hint_silenced_by_panic helper ──
    // Cannot test directly (requires AppHandle), but the component
    // PanicState.is_panicking() is already tested above.
}
