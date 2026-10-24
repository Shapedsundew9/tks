//! Socratic facilitator conversation provider trait and Ollama offload (D-130, D-EA-6, §6.4).

use serde_json::Value;
use std::time::Duration;

/// Facilitator conversation context assembled for proposal generation.
#[derive(Debug, Clone)]
pub struct FacilitatorContext {
    pub system_prompt: String,
    pub gap_questions: Vec<String>,
    pub turns: Vec<(String, String)>, // (role, text)
}

/// Provider trait for proposing facilitator Socratic questions.
pub trait FacilitatorProvider: Send + Sync {
    /// Proposes the next neutral Socratic question.
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;
}

/// No-op provider when facilitator assistance is disabled (`none`).
#[derive(Debug, Clone, Default)]
pub struct NoneFacilitatorProvider;

impl FacilitatorProvider for NoneFacilitatorProvider {
    fn propose(
        &self,
        _context: FacilitatorContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
        Box::pin(async { Err("facilitator provider is configured to 'none'".to_string()) })
    }
}

/// Local Ollama facilitator provider over native HTTP (`POST /api/chat`).
#[derive(Debug, Clone)]
pub struct OllamaFacilitatorProvider {
    pub client: reqwest::Client,
    pub base_url: String,
    pub model: String,
}

impl OllamaFacilitatorProvider {
    #[must_use]
    pub fn new(base_url: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url,
            model,
        }
    }
}

impl FacilitatorProvider for OllamaFacilitatorProvider {
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let model = self.model.clone();

        Box::pin(async move {
            let mut messages = Vec::new();
            messages.push(serde_json::json!({
                "role": "system",
                "content": context.system_prompt,
            }));

            if !context.gap_questions.is_empty() {
                let gaps_text = context
                    .gap_questions
                    .iter()
                    .map(|q| format!("- {q}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": format!(
                        "Open questions from previous cycles to explore (these are not established facts):\n{gaps_text}"
                    ),
                }));
            }

            for (role, text) in context.turns {
                let ollama_role = if role == "stakeholder" {
                    "user"
                } else {
                    "assistant"
                };
                messages.push(serde_json::json!({
                    "role": ollama_role,
                    "content": text,
                }));
            }

            let body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "think": false,
            });

            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("HTTP request to Ollama failed: {e}"))?;

            if !resp.status().is_success() {
                return Err(format!(
                    "Ollama returned HTTP error status: {}",
                    resp.status()
                ));
            }

            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse Ollama JSON response: {e}"))?;

            let text = json
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .ok_or_else(|| "Ollama response missing message.content".to_string())?;

            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err("Ollama returned empty response".to_string());
            }

            Ok(trimmed.to_string())
        })
    }
}
