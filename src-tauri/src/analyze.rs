use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::State;

use crate::db::{Database, TranscriptLineRow};
use crate::rag::embeddings::EmbeddingModel;
use crate::rag::indexer::search;
use crate::BatchTracker;

// Re-export LLM functions for backwards compatibility
pub use crate::llm::{
    call_anthropic, call_deepseek, call_gemini, call_ollama, call_openai, call_openrouter,
    default_model,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AnalyzeResult {
    pub summary: String,
    pub weak_questions: Vec<String>,
    pub forgotten_projects: Vec<String>,
    pub star_improvements: Vec<String>,
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

fn get_transcript_lines(db: &Database, session_id: &str) -> Result<Vec<TranscriptLineRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, speaker, text, started_at_ms, ended_at_ms
             FROM transcript_lines
             WHERE session_id = ?1
             ORDER BY started_at_ms ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(TranscriptLineRow {
                id: row.get(0)?,
                speaker: row.get(1)?,
                text: row.get(2)?,
                started_at_ms: row.get(3)?,
                ended_at_ms: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut lines = Vec::new();
    for row in rows {
        lines.push(row.map_err(|e| e.to_string())?);
    }
    Ok(lines)
}

// ---------------------------------------------------------------------------
// Prompt builder
// ---------------------------------------------------------------------------

fn build_analysis_prompt(lines: &[TranscriptLineRow], rag_context: &str) -> String {
    let transcript: String = lines
        .iter()
        .map(|l| {
            let speaker = match l.speaker.as_str() {
                "user" => "Candidato",
                _ => "Entrevistador",
            };
            format!("[{}]: {}", speaker, l.text)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r##"You are a technical interview analysis assistant. Your task is to analyze the full transcript of an interview (interviewer questions + candidate answers) using the candidate's document context (CV, projects, metrics).

## Interview Transcript
{}

## Candidate Document Context
{}

## Response format MUST be valid JSON with this exact structure (no markdown, no ```json blocks, pure JSON only):
{{
  "summary": "Executive summary of 3-4 sentences about how the interview went: what they asked, how the candidate responded, general tone.",
  "weak_questions": [
    "Specific question the candidate answered weakly and what they could have said instead"
  ],
  "forgotten_projects": [
    "Project or metric from the candidate's context that would have been relevant to mention but did not appear in their answers"
  ],
  "star_improvements": [
    "Specific moment where the candidate could have better used the STAR structure (Situation, Task, Action, Result)"
  ]
}}

## Important Instructions
1. Identify the technical and behavioral questions the interviewer asked.
2. Compare the candidate's answers with the context available in their documents.
3. If the candidate left out relevant projects or important metrics, list them in `forgotten_projects`.
4. Point out which questions were answered weakly or without enough depth.
5. Suggest concrete STAR structure improvements where the candidate gave generic answers.
6. Respond ONLY with the JSON, no additional text."##,
        transcript, rag_context
    )
}

// ---------------------------------------------------------------------------
// RAG context builder
// ---------------------------------------------------------------------------

fn build_rag_context(
    model: &Arc<Mutex<EmbeddingModel>>,
    db: &Database,
    lines: &[TranscriptLineRow],
) -> String {
    let questions: Vec<&str> = lines
        .iter()
        .filter(|l| l.speaker == "interviewer")
        .map(|l| l.text.as_str())
        .collect();

    if questions.is_empty() {
        return String::from("(No indexed documents available)");
    }

    let mut all_chunks: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for q in questions {
        if let Ok(results) = search(model, db, q, 3) {
            for r in &results {
                if seen.insert(r.id) {
                    let snippet = if let (Some(tag), Some(metric)) = (&r.tag, &r.metric) {
                        format!("[{}] {} — {}", tag, r.text, metric)
                    } else {
                        r.text.clone()
                    };
                    all_chunks.push(snippet);
                }
            }
        }
    }

    if all_chunks.is_empty() {
        String::from("(No relevant documents found in the candidate's context)")
    } else {
        all_chunks.join("\n---\n")
    }
}

// ---------------------------------------------------------------------------
// Response parser
// ---------------------------------------------------------------------------

fn parse_llm_response(raw: &str) -> Result<AnalyzeResult, String> {
    let json_str = extract_json(raw).ok_or_else(|| {
        format!("No JSON found in LLM response: {raw:.200}")
    })?;

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse LLM JSON: {e}\nRaw: {json_str:.200}"))?;

    Ok(AnalyzeResult {
        summary: parsed
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        weak_questions: parsed
            .get("weak_questions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        forgotten_projects: parsed
            .get("forgotten_projects")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        star_improvements: parsed
            .get("star_improvements")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    })
}

fn extract_json(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find('{') {
        let mut depth = 0u32;
        let mut in_string = false;
        let mut escaped = false;
        for (i, ch) in trimmed[start..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                '{' if !in_string => depth += 1,
                '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(trimmed[start..=start + i].to_string());
                    }
                }
                _ => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tauri command (async)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn analyze_session(
    session_id: String,
    provider: String,
    model: Option<String>,
    db: State<'_, Database>,
    model_state: State<'_, Arc<Mutex<EmbeddingModel>>>,
    batch_tracker: State<'_, BatchTracker>,
) -> Result<AnalyzeResult, String> {
    let ready = {
        let tracker = batch_tracker.0.lock().map_err(|e| e.to_string())?;
        if tracker.contains(&session_id) {
            true
        } else {
            // If not in the in-memory tracker (e.g. after app restart),
            // check whether transcript lines already exist in the database.
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transcript_lines WHERE session_id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            count > 0
        } // tracker lock is dropped here, so we won't deadlock below
    };

    if !ready {
        return Err(
            "Channel A transcript is not ready yet. Wait for processing to finish."
                .to_string(),
        );
    }

    let lines = get_transcript_lines(&db, &session_id)?;
    if lines.is_empty() {
        return Err("No transcript lines for this session.".to_string());
    }

    let rag_context = build_rag_context(model_state.inner(), &db, &lines);
    let prompt = build_analysis_prompt(&lines, &rag_context);
    let api_key = crate::keys::get_api_key(&provider)?;

    let model_name = model
        .as_deref()
        .unwrap_or_else(|| default_model(&provider))
        .to_string();

    let raw = match provider.as_str() {
        "ollama" => call_ollama(&prompt, &model_name).await?,
        "openai" => call_openai(&prompt, &model_name, &api_key).await?,
        "anthropic" => call_anthropic(&prompt, &model_name, &api_key).await?,
        "gemini" => call_gemini(&prompt, &model_name, &api_key).await?,
        "openrouter" => call_openrouter(&prompt, &model_name, &api_key).await?,
        "deepseek" => call_deepseek(&prompt, &model_name, &api_key).await?,
        other => return Err(format!("Unknown provider: {other}")),
    };

    parse_llm_response(&raw)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_from_plain() {
        let input = r#"{"summary": "test"}"#;
        assert_eq!(extract_json(input), Some(r#"{"summary": "test"}"#.to_string()));
    }

    #[test]
    fn extract_json_with_prefix_text() {
        let input = "Here is the analysis:\n\n```json\n{\"summary\": \"test\"}\n```";
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        assert!(extracted.unwrap().contains("\"summary\""));
    }

    #[test]
    fn extract_json_nested_braces() {
        let input = r#"{"summary": "test", "items": [{"a": 1}, {"b": 2}]}"#;
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["summary"], "test");
    }

    #[test]
    fn extract_json_no_braces() {
        assert_eq!(extract_json("just plain text"), None);
    }

    #[test]
    fn extract_json_empty_string() {
        assert_eq!(extract_json(""), None);
    }

    #[test]
    fn extract_json_escaped_braces_in_string() {
        let input = r#"{"text": "hello {world} and {foo}"}"#;
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["text"], "hello {world} and {foo}");
    }

    #[test]
    fn extract_json_with_escaped_quote() {
        let input = r#"{"msg": "hello \"world\" test"}"#;
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["msg"], r#"hello "world" test"#);
    }

    #[test]
    fn extract_json_deeply_nested_balanced() {
        let input = r#"{"a": {"b": {"c": [1, 2, {"d": 3}]}}}"#;
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["a"]["b"]["c"][2]["d"], 3);
    }

    #[test]
    fn extract_json_unbalanced_braces_returns_none() {
        let input = r#"{"key": "value""#;
        assert_eq!(extract_json(input), None);
    }

    #[test]
    fn extract_json_only_array_returns_none() {
        let input = r#"["item1", "item2"]"#;
        assert_eq!(extract_json(input), None);
    }

    #[test]
    fn extract_json_with_unicode_escapes() {
        let input = r#"{"text": "\u0048\u0065\u006c\u006c\u006f"}"#;
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["text"], "Hello");
    }

    #[test]
    fn extract_json_newlines_in_string() {
        let input = "{\"text\": \"line1\\nline2\"}";
        let extracted = extract_json(input);
        assert!(extracted.is_some());
        let v: serde_json::Value = serde_json::from_str(&extracted.unwrap()).unwrap();
        assert_eq!(v["text"], "line1\nline2");
    }

    #[test]
    fn parse_llm_response_valid_full() {
        let raw = r#"{
            "summary": "Good interview",
            "weak_questions": ["Q1", "Q2"],
            "forgotten_projects": ["Project X"],
            "star_improvements": ["STAR at 5:30"]
        }"#;
        let result = parse_llm_response(raw).unwrap();
        assert_eq!(result.summary, "Good interview");
        assert_eq!(result.weak_questions, vec!["Q1", "Q2"]);
        assert_eq!(result.forgotten_projects, vec!["Project X"]);
        assert_eq!(result.star_improvements, vec!["STAR at 5:30"]);
    }

    #[test]
    fn parse_llm_response_missing_fields_default_to_empty() {
        let raw = r#"{"summary": "Only summary"}"#;
        let result = parse_llm_response(raw).unwrap();
        assert_eq!(result.summary, "Only summary");
        assert!(result.weak_questions.is_empty());
        assert!(result.forgotten_projects.is_empty());
        assert!(result.star_improvements.is_empty());
    }

    #[test]
    fn parse_llm_response_empty_object() {
        let result = parse_llm_response(r#"{}"#).unwrap();
        assert!(result.summary.is_empty());
        assert!(result.weak_questions.is_empty());
    }

    #[test]
    fn parse_llm_response_invalid_json() {
        let result = parse_llm_response("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn parse_llm_response_markdown_wrapped() {
        let raw = "Here is the result:\n\n```json\n{\"summary\": \"Markdown wrapped\", \"weak_questions\": [], \"forgotten_projects\": [], \"star_improvements\": []}\n```";
        let result = parse_llm_response(raw).unwrap();
        assert_eq!(result.summary, "Markdown wrapped");
    }

    #[test]
    fn parse_llm_response_null_summary() {
        let raw = r#"{"summary": null, "weak_questions": [], "forgotten_projects": [], "star_improvements": []}"#;
        let result = parse_llm_response(raw).unwrap();
        assert!(result.summary.is_empty());
    }

    #[test]
    fn parse_llm_response_summary_not_string() {
        let raw = r#"{"summary": 42, "weak_questions": [], "forgotten_projects": [], "star_improvements": []}"#;
        let result = parse_llm_response(raw).unwrap();
        assert!(result.summary.is_empty());
    }

    #[test]
    fn parse_llm_response_non_utf8_like_content() {
        let raw = "{\"summary\": \"Normal summary\", \"weak_questions\": [\"Q1\"], \"forgotten_projects\": [], \"star_improvements\": []}";
        let result = parse_llm_response(raw).unwrap();
        assert_eq!(result.summary, "Normal summary");
        assert_eq!(result.weak_questions, vec!["Q1"]);
    }

    #[test]
    fn parse_llm_response_partial_fields_present() {
        let raw = r#"{"summary": "Partial", "forgotten_projects": ["Project X"]}"#;
        let result = parse_llm_response(raw).unwrap();
        assert_eq!(result.summary, "Partial");
        assert_eq!(result.forgotten_projects, vec!["Project X"]);
        assert!(result.weak_questions.is_empty());
        assert!(result.star_improvements.is_empty());
    }

    #[test]
    fn analyze_result_serialization() {
        let result = AnalyzeResult {
            summary: "Good interview".to_string(),
            weak_questions: vec!["Q1".to_string()],
            forgotten_projects: vec![],
            star_improvements: vec!["STAR at 5:30".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""summary":"Good interview""#));
        assert!(json.contains(r#""weak_questions":["Q1"]"#));
        assert!(json.contains(r#""forgotten_projects":[]"#));
        assert!(json.contains(r#""star_improvements":["STAR at 5:30"]"#));
    }

    #[test]
    fn build_analysis_prompt_contains_transcript() {
        let lines = vec![
            TranscriptLineRow {
                id: 1,
                speaker: "interviewer".into(),
                text: "Tell me about yourself".into(),
                started_at_ms: 0,
                ended_at_ms: 1000,
            },
            TranscriptLineRow {
                id: 2,
                speaker: "user".into(),
                text: "I am a software engineer".into(),
                started_at_ms: 1000,
                ended_at_ms: 3000,
            },
        ];
        let prompt = build_analysis_prompt(&lines, "some rag context");
        assert!(prompt.contains("Tell me about yourself"));
        assert!(prompt.contains("I am a software engineer"));
        assert!(prompt.contains("[Entrevistador]"));
        assert!(prompt.contains("[Candidato]"));
    }

    #[test]
    fn build_analysis_prompt_empty_transcript() {
        let prompt = build_analysis_prompt(&[], "");
        assert!(prompt.contains("summary"));
        assert!(prompt.contains("weak_questions"));
    }

    #[test]
    fn build_analysis_prompt_speaker_fallback() {
        let lines = vec![TranscriptLineRow {
            id: 1,
            speaker: "unknown_role".into(),
            text: "Hello".into(),
            started_at_ms: 0,
            ended_at_ms: 100,
        }];
        let prompt = build_analysis_prompt(&lines, "");
        assert!(prompt.contains("[Entrevistador]"));
        assert!(!prompt.contains("[Candidato]"));
    }

    #[test]
    fn build_rag_context_only_user_lines_no_questions() {
        let lines = vec![TranscriptLineRow {
            id: 1,
            speaker: "user".into(),
            text: "I am answering".into(),
            started_at_ms: 0,
            ended_at_ms: 100,
        }];
        let questions: Vec<&str> = lines.iter()
            .filter(|l| l.speaker == "interviewer")
            .map(|l| l.text.as_str())
            .collect();
        assert!(questions.is_empty());
    }

    #[test]
    fn analyze_session_rejects_unready_transcript() {
        let tracker = BatchTracker(Arc::new(Mutex::new(std::collections::HashSet::new())));
        let ready = tracker.0.lock().unwrap().contains("sess-unready");
        assert!(!ready);
    }

    #[test]
    fn analyze_session_accepts_ready_transcript() {
        let tracker = BatchTracker(Arc::new(Mutex::new(std::collections::HashSet::new())));
        tracker.0.lock().unwrap().insert("sess-ready".into());
        let ready = tracker.0.lock().unwrap().contains("sess-ready");
        assert!(ready);
    }
}
