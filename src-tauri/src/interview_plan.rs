use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::Database;
use crate::rag::embeddings::EmbeddingModel;
use crate::rag::indexer::search;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedQuestion {
    pub text: String,
    pub qtype: String,
    pub budget_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewPlan {
    pub questions: Vec<PlannedQuestion>,
}

// ---------------------------------------------------------------------------
// RAG context builder
// ---------------------------------------------------------------------------

fn build_rag_context_for_jd(
    model: &Arc<Mutex<EmbeddingModel>>,
    db: &Database,
    job_description: &str,
) -> String {
    match search(model, db, job_description, 5) {
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
        _ => String::from("(No relevant documents indexed for this position)"),
    }
}

// ---------------------------------------------------------------------------
// Prompt builder
// ---------------------------------------------------------------------------

fn build_interview_plan_prompt(
    job_description: &str,
    duration_minutes: u32,
    rag_context: &str,
) -> String {
    let total_seconds = duration_minutes * 60;

    format!(
        r##"You are a senior technical interviewer. Create a structured interview plan for the following position.

## Job Description
{job_description}

## Candidate context (indexed docs)
{rag_context}

## Instructions
1. Generate a list of realistic questions for this position.
2. Mix question types: technical (qtype: "technical"), behavioral/STAR (qtype: "star"), architecture (qtype: "architecture"), and a trick question or two (qtype: "trap").
3. Assign a time budget (budget_seconds) to each question. The total must be approximately {total_seconds} seconds (={duration_minutes} minutes).
4. Respond ONLY in valid JSON format (no markdown, no ```json blocks) with this exact structure:
{{
  "questions": [
    {{"text": "Explain how you would design a distributed cache system", "qtype": "architecture", "budget_seconds": 180}},
    {{"text": "Tell me about a technical conflict you resolved", "qtype": "star", "budget_seconds": 120}}
  ]
}}"##,
    )
}

// ---------------------------------------------------------------------------
// Response parser
// ---------------------------------------------------------------------------

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

fn parse_interview_plan(raw: &str) -> Result<InterviewPlan, String> {
    let json_str = extract_json(raw)
        .ok_or_else(|| format!("No JSON found in LLM response: {raw:.200}"))?;

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse LLM JSON: {e}\nRaw: {json_str:.200}"))?;

    let questions = parsed
        .get("questions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|q| {
                    let text = q.get("text")?.as_str()?.to_string();
                    let qtype = q
                        .get("qtype")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_lowercase())
                        .unwrap_or_else(|| "technical".to_string());
                    let budget_seconds = q
                        .get("budget_seconds")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(120) as u32;
                    Some(PlannedQuestion {
                        text,
                        qtype,
                        budget_seconds,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if questions.is_empty() {
        return Err("LLM returned no questions in the plan".to_string());
    }

    Ok(InterviewPlan { questions })
}

// ---------------------------------------------------------------------------
// Tauri command (async)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn generate_interview_plan(
    job_description: String,
    duration_minutes: u32,
    provider: String,
    model: Option<String>,
    db: State<'_, Database>,
    model_state: State<'_, Arc<Mutex<EmbeddingModel>>>,
) -> Result<InterviewPlan, String> {
    if job_description.trim().is_empty() {
        return Err("Job description cannot be empty".to_string());
    }

    let api_key = crate::keys::get_api_key(&provider)
        .map_err(|_| format!("No API key configured for provider '{provider}'. Configure it in the onboarding or settings."))?;

    let model_name = model
        .as_deref()
        .unwrap_or_else(|| crate::llm::default_model(&provider))
        .to_string();

    let rag_context = build_rag_context_for_jd(model_state.inner(), db.inner(), &job_description);
    let prompt = build_interview_plan_prompt(&job_description, duration_minutes, &rag_context);

    let raw = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        async {
            match provider.as_str() {
                "ollama" => crate::llm::call_ollama(&prompt, &model_name).await,
                "openai" => crate::llm::call_openai(&prompt, &model_name, &api_key).await,
                "anthropic" => crate::llm::call_anthropic(&prompt, &model_name, &api_key).await,
                "gemini" => crate::llm::call_gemini(&prompt, &model_name, &api_key).await,
                "openrouter" => crate::llm::call_openrouter(&prompt, &model_name, &api_key).await,
                "deepseek" => crate::llm::call_deepseek(&prompt, &model_name, &api_key).await,
                other => Err(format!("Unknown provider: {other}")),
            }
        },
    )
    .await
    .map_err(|_| format!("Interview plan generation timed out after 120s. The model may be too slow or the job description too long. Try a smaller model or a shorter duration."))??;

    parse_interview_plan(&raw)
}
