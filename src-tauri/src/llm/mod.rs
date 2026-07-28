use std::sync::OnceLock;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Response structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct OpenAIResponse {
    #[serde(default)]
    pub choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
pub struct OpenAIChoice {
    #[serde(default)]
    pub message: OpenAIMessage,
}

#[derive(Default, Deserialize)]
pub struct OpenAIMessage {
    #[serde(default)]
    pub content: String,
}

#[derive(Deserialize)]
pub struct AnthropicResponse {
    #[serde(default)]
    pub content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
pub struct AnthropicContent {
    #[serde(rename = "type")]
    pub _type: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize)]
pub struct GeminiResponse {
    #[serde(default)]
    pub candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
pub struct GeminiCandidate {
    #[serde(default)]
    pub content: GeminiContent,
}

#[derive(Default, Deserialize)]
pub struct GeminiContent {
    #[serde(default)]
    pub parts: Vec<GeminiPart>,
}

#[derive(Deserialize)]
pub struct GeminiPart {
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize)]
pub struct OllamaResponse {
    #[serde(default)]
    pub message: Option<OllamaMessage>,
}

#[derive(Deserialize)]
pub struct OllamaMessage {
    #[serde(default)]
    pub content: String,
}

// ---------------------------------------------------------------------------
// Shared HTTP client
// ---------------------------------------------------------------------------

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::new())
}

// ---------------------------------------------------------------------------
// Provider API calls
// ---------------------------------------------------------------------------

pub async fn call_ollama(prompt: &str, model: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "stream": false,
    });

    let resp = http_client()
        .post("http://localhost:11434/api/chat")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                format!(
                    "Ollama is not running at http://localhost:11434. \
                     Start it with 'ollama serve' and make sure the model '{}' is pulled.",
                    model
                )
            } else if e.is_timeout() {
                format!(
                    "Ollama timed out. The model '{}' may be loading or the request is too large. \
                     Try again in a few seconds.",
                    model
                )
            } else {
                format!("Ollama request failed: {e}")
            }
        })?;

    let data: OllamaResponse = resp
        .json()
        .await
        .map_err(|e| format!("Ollama response parse failed: {e}"))?;

    data.message
        .map(|m| m.content)
        .ok_or_else(|| "Ollama returned no message content".to_string())
}

pub async fn call_openai(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "Eres un asistente de análisis de entrevistas técnicas. Siempre respondes ÚNICAMENTE con JSON válido, sin texto adicional ni markdown."},
            {"role": "user", "content": prompt}
        ],
    });

    let resp = http_client()
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI request failed: {e}"))?;

    let data: OpenAIResponse = resp
        .json()
        .await
        .map_err(|e| format!("OpenAI response parse failed: {e}"))?;

    data.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| "OpenAI returned no choices".to_string())
}

pub async fn call_anthropic(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "system": "Eres un asistente de análisis de entrevistas técnicas. Siempre respondes ÚNICAMENTE con JSON válido, sin texto adicional ni markdown.",
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "max_tokens": 4096,
    });

    let resp = http_client()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Anthropic request failed: {e}"))?;

    let data: AnthropicResponse = resp
        .json()
        .await
        .map_err(|e| format!("Anthropic response parse failed: {e}"))?;

    data.content
        .into_iter()
        .next()
        .map(|c| c.text)
        .ok_or_else(|| "Anthropic returned no content".to_string())
}

pub async fn call_gemini(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let body = serde_json::json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }],
        "generationConfig": {
            "responseMimeType": "application/json"
        }
    });

    let resp = http_client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini request failed: {e}"))?;

    let data: GeminiResponse = resp
        .json()
        .await
        .map_err(|e| format!("Gemini response parse failed: {e}"))?;

    data.candidates
        .into_iter()
        .next()
        .and_then(|c| c.content.parts.into_iter().next())
        .map(|p| p.text)
        .ok_or_else(|| "Gemini returned no candidates".to_string())
}

pub async fn call_openrouter(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "Eres un asistente de análisis de entrevistas técnicas. Siempre respondes ÚNICAMENTE con JSON válido, sin texto adicional ni markdown."},
            {"role": "user", "content": prompt}
        ],
    });

    let resp = http_client()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenRouter request failed: {e}"))?;

    let data: OpenAIResponse = resp
        .json()
        .await
        .map_err(|e| format!("OpenRouter response parse failed: {e}"))?;

    data.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| "OpenRouter returned no choices".to_string())
}

pub async fn call_deepseek(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "Eres un asistente de análisis de entrevistas técnicas. Siempre respondes ÚNICAMENTE con JSON válido, sin texto adicional ni markdown."},
            {"role": "user", "content": prompt}
        ],
    });

    let resp = http_client()
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("DeepSeek request failed: {e}"))?;

    let data: OpenAIResponse = resp
        .json()
        .await
        .map_err(|e| format!("DeepSeek response parse failed: {e}"))?;

    data.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| "DeepSeek returned no choices".to_string())
}

// ---------------------------------------------------------------------------
// Default models per provider
// ---------------------------------------------------------------------------

pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "ollama" => "llama3",
        "openai" => "gpt-4o",
        "anthropic" => "claude-3-5-sonnet-20241022",
        "gemini" => "gemini-1.5-pro",
        "openrouter" => "openai/gpt-4o",
        "deepseek" => "deepseek-chat",
        _ => "gpt-4o",
    }
}

// ---------------------------------------------------------------------------
// Hint generation (for live interview coaching)
// ---------------------------------------------------------------------------

/// Generates a short, actionable hint using the configured LLM provider.
///
/// The prompt combines the interviewer's question with RAG context (relevant
/// chunks from the candidate's indexed documents).
pub async fn generate_hint(
    question: &str,
    rag_context: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<String, String> {
    let prompt = format!(
        "You are a technical interview coach. The interviewer asked: \"{question}\". \
         Relevant candidate context: {rag_context}. \
         Respond with ONE hint of max 25 words, concrete and actionable, no extra explanation."
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(4),
        call_provider(&prompt, provider, model, api_key),
    )
    .await
    .map_err(|_| "Hint generation timed out after 4s".to_string())?
}

async fn call_provider(
    prompt: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<String, String> {
    match provider {
        "ollama" => call_ollama(prompt, model).await,
        "openai" => call_openai(prompt, model, api_key).await,
        "anthropic" => call_anthropic(prompt, model, api_key).await,
        "gemini" => call_gemini(prompt, model, api_key).await,
        "openrouter" => call_openrouter(prompt, model, api_key).await,
        "deepseek" => call_deepseek(prompt, model, api_key).await,
        other => Err(format!("Unknown provider: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_for_known_providers() {
        assert_eq!(default_model("ollama"), "llama3");
        assert_eq!(default_model("openai"), "gpt-4o");
        assert_eq!(default_model("anthropic"), "claude-3-5-sonnet-20241022");
        assert_eq!(default_model("gemini"), "gemini-1.5-pro");
        assert_eq!(default_model("openrouter"), "openai/gpt-4o");
        assert_eq!(default_model("deepseek"), "deepseek-chat");
    }

    #[test]
    fn default_model_unknown_falls_back_to_gpt4o() {
        assert_eq!(default_model("unknown-provider"), "gpt-4o");
    }

    #[test]
    fn http_client_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<reqwest::Client>();
    }
}
