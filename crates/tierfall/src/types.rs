//! OpenAI-compatible chat types.
//!
//! Every struct keeps unknown fields in a flattened `extra` map so that provider features such as
//! `tools`, `tool_choice`, `response_format`, `reasoning_effort`, or `tool_calls` pass through
//! unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::provider::ProviderKind;

/// A chat completion request (`POST /v1/chat/completions`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatRequest {
    /// Route alias (`auto`, `fast`, …), concrete model ID, or `model@account`.
    #[serde(default)]
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ChatRequest {
    /// Creates an empty request targeting `model` (an alias, model ID, or `model@account`).
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Self::default()
        }
    }

    /// Appends a system message.
    #[must_use]
    pub fn system(self, text: impl Into<String>) -> Self {
        self.message(Message::system(text))
    }

    /// Appends a user message.
    #[must_use]
    pub fn user(self, text: impl Into<String>) -> Self {
        self.message(Message::user(text))
    }

    /// Appends an assistant message.
    #[must_use]
    pub fn assistant(self, text: impl Into<String>) -> Self {
        self.message(Message::assistant(text))
    }

    /// Appends an arbitrary message.
    #[must_use]
    pub fn message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    /// Sets any additional OpenAI parameter (e.g. `temperature`, `tools`, `response_format`).
    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }
}

/// A chat message. `content` may be a string, an array of content parts, or absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Message {
    fn with_text(role: &str, text: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: Some(Value::String(text.into())),
            extra: Map::new(),
        }
    }

    #[must_use]
    pub fn system(text: impl Into<String>) -> Self {
        Self::with_text("system", text)
    }

    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self::with_text("user", text)
    }

    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::with_text("assistant", text)
    }

    /// Returns the textual content, concatenating `text` parts when content is an array.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        match self.content.as_ref()? {
            Value::String(s) => Some(s.clone()),
            Value::Array(parts) => {
                let joined: String = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect();
                Some(joined)
            }
            _ => None,
        }
    }
}

/// Token usage reported by the upstream provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Usage {
    /// Total tokens, falling back to prompt + completion.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total_tokens.unwrap_or_else(|| {
            self.prompt_tokens.unwrap_or(0) + self.completion_tokens.unwrap_or(0)
        })
    }
}

/// One choice in a non-streaming response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Choice {
    #[serde(default)]
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Which account and model actually served a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServedBy {
    pub account: String,
    pub provider: ProviderKind,
    pub model: String,
    /// Number of upstream calls made (including the successful one).
    pub attempts: u32,
}

/// A non-streaming chat completion response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    /// Populated by tierfall; not part of the upstream JSON.
    #[serde(skip)]
    pub served_by: Option<ServedBy>,
}

impl ChatResponse {
    /// Text of the first choice, if any.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.choices.first()?.message.as_ref()?.text()
    }
}

/// The incremental delta of a streaming chunk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Delta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One choice inside a streaming chunk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChunkChoice {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub delta: Delta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A streaming chat completion chunk (`object: chat.completion.chunk`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatChunk {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ChatChunk {
    /// Text content of the first choice's delta, if any.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.choices.first()?.delta.content.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips_unknown_fields() {
        let raw = json!({
            "model": "auto",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hi"}]},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "1"}]},
                {"role": "tool", "tool_call_id": "1", "content": "42"}
            ],
            "tools": [{"type": "function", "function": {"name": "f"}}],
            "response_format": {"type": "json_object"},
            "temperature": 0.2
        });
        let req: ChatRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(req.messages[0].text().as_deref(), Some("hi"));
        assert!(req.extra.contains_key("tools"));
        let back = serde_json::to_value(&req).unwrap();
        assert_eq!(back["tools"], raw["tools"]);
        assert_eq!(back["response_format"], raw["response_format"]);
        assert_eq!(
            back["messages"][1]["tool_calls"],
            raw["messages"][1]["tool_calls"]
        );
        assert_eq!(back["messages"][2]["tool_call_id"], "1");
    }

    #[test]
    fn builder_and_response_text() {
        let req = ChatRequest::new("fast")
            .system("be brief")
            .user("hello")
            .param("temperature", 0.5);
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.extra["temperature"], 0.5);

        let resp: ChatResponse = serde_json::from_value(json!({
            "id": "x", "object": "chat.completion", "created": 1, "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "OK"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        }))
        .unwrap();
        assert_eq!(resp.text().as_deref(), Some("OK"));
        assert_eq!(resp.usage.unwrap().total(), 4);
    }

    #[test]
    fn chunk_text() {
        let chunk: ChatChunk = serde_json::from_value(json!({
            "id": "x", "object": "chat.completion.chunk", "model": "m",
            "choices": [{"index": 0, "delta": {"content": "Hel"}}]
        }))
        .unwrap();
        assert_eq!(chunk.text(), Some("Hel"));
    }
}
