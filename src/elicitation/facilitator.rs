//! Socratic facilitator conversation provider trait and Ollama offload (D-130, D-EA-6, §6.4).

use serde_json::Value;
use std::time::Duration;

/// Facilitator conversation context assembled for proposal generation.
#[derive(Debug, Clone, Default)]
pub struct FacilitatorContext {
    pub system_prompt: String,
    pub session_title: String,
    pub is_inspiration: bool,
    pub focus_topic: Option<String>,
    pub gap_questions: Vec<String>,
    pub explored_items: Vec<String>,
    pub turns: Vec<(String, String)>, // (role, text)
}

/// Extracts concise idea titles or questions from a generated facilitator proposal
/// so they can be added to the session's `explored_items` memory.
#[must_use]
pub fn extract_proposal_items(text: &str, is_inspiration: bool) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if !is_inspiration {
        return vec![trimmed.to_string()];
    }

    let mut items = Vec::new();
    for block in trimmed.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        // Extract bold title `**Title**:` if present, otherwise first sentence/line
        if let Some(start) = block.find("**")
            && let Some(end_rel) = block[start + 2..].find("**")
        {
            let title = block[start + 2..start + 2 + end_rel]
                .trim()
                .trim_end_matches(':')
                .trim();
            if !title.is_empty() {
                items.push(title.to_string());
                continue;
            }
        }
        let fallback: String = block
            .lines()
            .next()
            .unwrap_or(block)
            .chars()
            .take(80)
            .collect();
        if !fallback.trim().is_empty() {
            items.push(fallback.trim().to_string());
        }
    }
    items
}

/// Assembles the structured user analysis prompt from the [`FacilitatorContext`].
#[must_use]
pub fn build_facilitator_user_prompt(context: &FacilitatorContext) -> String {
    let mut sections = Vec::new();

    if !context.session_title.trim().is_empty() {
        sections.push(format!(
            "Session Goal / Title: {}",
            context.session_title.trim()
        ));
    }

    let (scope_boundaries, regular_turns): (Vec<_>, Vec<_>) = context
        .turns
        .iter()
        .enumerate()
        .partition(|(_, (_, text))| {
            let lower = text.trim().to_lowercase();
            lower.starts_with("out of scope") || lower.starts_with("boundary:")
        });

    if !scope_boundaries.is_empty() {
        let boundaries_text = scope_boundaries
            .iter()
            .map(|(idx, (role, text))| format!("- [t{}] ({role}): {text}", idx + 1))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "Explicit Project Scope Boundaries & Exclusions (These are strictly OUT OF SCOPE. Do NOT propose, question, or assume these mechanisms):\n{boundaries_text}"
        ));
    }

    if regular_turns.is_empty() {
        sections.push(
            "Established Requirements & Statements So Far:\n(No statements recorded yet — session is just beginning.)"
                .to_string(),
        );
    } else {
        let mut lines = Vec::with_capacity(regular_turns.len());
        for (idx, (role, text)) in regular_turns {
            let turn_num = idx + 1;
            lines.push(format!("[t{turn_num}] ({role}): {text}"));
        }
        sections.push(format!(
            "Established Requirements & Statements So Far:\n{}",
            lines.join("\n")
        ));
    }

    if !context.gap_questions.is_empty() {
        let gaps_text = context
            .gap_questions
            .iter()
            .map(|q| format!("- {q}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "Open Gap Questions from Prior Elicitation Cycle:\n{gaps_text}"
        ));
    }

    if !context.explored_items.is_empty() {
        let explored_text = context
            .explored_items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "Already Explored / Excluded Ideas & Questions (DO NOT repeat, rephrase, or overlap with these):\n{explored_text}"
        ));
    }

    let directive = if context.is_inspiration {
        if let Some(ref topic) = context.focus_topic {
            format!(
                "Task:\n\
                 The stakeholder specifically wants to explore domain rules, invariants, and edge cases regarding: \"{topic}\".\n\
                 Review the established statements above so you understand the full domain context, then propose 1 to 3 concrete, non-overlapping domain requirement statements regarding \"{topic}\" that establish unexamined preconditions, exceptions, or authority boundaries not already stated or listed in the excluded items."
            )
        } else {
            "Task:\n\
             Step 1: Check the 3 Foundation Anchors (Real-world outcome, Primary actors/roles, and Core operational scope/invariants).\n\
             - If ANY anchor is indeterminate or unstated, propose 1 to 3 concrete domain statements to establish the missing foundational boundaries.\n\
             - If all 3 anchors are established, audit the 5 orthogonal domain axes (1. Agency & Responsibility, 2. Preconditions & Invariants, 3. Exception & Conflict Resolution, 4. Temporal Cadence & Lifecycle, 5. Evidence & Accountability). Note: any axis addressed or explicitly declared Out of Scope counts as covered. Spot the most important gaps in coverage across the axis with zero or lowest coverage.\n\
             Step 2: Propose 1 to 3 concrete domain requirement statements that nail down invariants, negative boundaries, or exception policies in that under-explored axis.\n\
             Do NOT repeat or rehash what is already covered in the transcript or the excluded list."
                .to_string()
        }
    } else if let Some(ref topic) = context.focus_topic {
        format!(
            "Task:\n\
             Focus on \"{topic}\". Ask ONE specific, high-entropy bisection question (Option A vs Option B) probing an unexamined operational boundary, exception scenario, or authority limit not yet answered in the transcript."
        )
    } else if context.turns.is_empty() {
        "Task:\n\
         Ask ONE specific, grounded opening Socratic bisection question to help the stakeholder establish the core real-world domain problem, who is involved, and the desired outcome."
            .to_string()
    } else {
        "Task:\n\
         Step 1: Foundation Anchors Check.\n\
         Review the established statements against the 3 Foundation Anchors:\n\
         - [A1: Core Operational Problem & Desired Outcome]\n\
         - [A2: Primary Stakeholders & Agency]\n\
         - [A3: Core Operational Invariant or Scope Boundary]\n\
         \n\
         If ANY anchor is indeterminate or missing:\n\
         Your mode is ANCHOR BISECTION.\n\
         Ask ONE high-entropy bisection question (Option A vs Option B) that resolves the missing foundation (e.g. bisecting the primary actors or clarifying the core operational boundary). Do NOT dive into obscure edge cases yet.\n\
         \n\
         If ALL 3 anchors are established:\n\
         Your mode is NEGATIVE-SPACE VOID-FILLING.\n\
         Audit the 5 orthogonal domain axes: (1. Agency & Responsibility, 2. Preconditions & Invariants, 3. Exception & Conflict Resolution, 4. Temporal Cadence & Lifecycle, 5. Evidence & Accountability).\n\
         Identify the axis with zero or lowest coverage. Ask ONE high-entropy bisection question presenting a concrete fork: \"If [Scenario X occurs in the least-covered axis], does the domain rule dictate Option A or Option B?\"\n\
         Do NOT ask generic boilerplate questions or repeat anything from the Already Explored list."
            .to_string()
    };

    sections.push(directive);
    sections.join("\n\n")
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

/// Resolve and normalize an Ollama URL (D-EA-6).
///
/// Handles:
/// - Empty or whitespace string: falls back to `http://ollama:11434` if resolvable (e.g. devcontainer),
///   otherwise `http://localhost:11434`.
/// - Host without scheme (e.g. `ollama:11434`, `localhost:11434`, `127.0.0.1:11434`): prepends `http://`.
/// - Trims trailing slashes.
#[must_use]
pub fn resolve_ollama_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        if is_ollama_host_resolvable() {
            return "http://ollama:11434".to_string();
        }
        return "http://localhost:11434".to_string();
    }

    let with_scheme = if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("http://{trimmed}")
    } else {
        trimmed.to_string()
    };

    with_scheme.trim_end_matches('/').to_string()
}

fn is_ollama_host_resolvable() -> bool {
    use std::net::ToSocketAddrs;
    ("ollama", 11434).to_socket_addrs().is_ok()
}

impl OllamaFacilitatorProvider {
    #[must_use]
    pub fn new(base_url: String, model: String) -> Self {
        let base_url = resolve_ollama_url(&base_url);
        let model = if model.trim().is_empty() {
            "qwen3:8b".to_string()
        } else {
            model.trim().to_string()
        };
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

fn log_llm_exchange(
    model: &str,
    is_inspiration: bool,
    focus_topic: Option<&str>,
    duration: std::time::Duration,
    system_prompt: &str,
    user_prompt: &str,
    response: Result<&str, &str>,
) {
    let mode = if is_inspiration {
        "inspiration"
    } else {
        "socratic"
    };
    let topic_display = focus_topic.unwrap_or("None");
    let duration_ms = duration.as_millis();
    let timestamp = chrono::Utc::now().to_rfc3339();

    match response {
        Ok(text) => {
            tracing::info!(
                target: "tks::llm",
                model = %model,
                mode = %mode,
                topic = %topic_display,
                duration_ms = duration_ms,
                response_len = text.len(),
                "LLM proposal exchange completed successfully"
            );
        }
        Err(err) => {
            tracing::warn!(
                target: "tks::llm",
                model = %model,
                mode = %mode,
                topic = %topic_display,
                duration_ms = duration_ms,
                error = %err,
                "LLM proposal exchange failed"
            );
        }
    }

    let log_path = if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home).join(".cache/tks/logs/llm_exchanges.log")
    } else {
        std::path::PathBuf::from("/tmp/tks_llm_exchanges.log")
    };

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let resp_body = match response {
            Ok(t) => t,
            Err(e) => e,
        };
        let status = if response.is_ok() { "SUCCESS" } else { "ERROR" };
        let _ = writeln!(
            file,
            "================================================================================\n\
             [{timestamp}] LLM Exchange (Model: {model}, Mode: {mode}, Status: {status})\n\
             Focus Topic: {topic_display} | Duration: {duration_ms}ms\n\
             --------------------------------------------------------------------------------\n\
             SYSTEM PROMPT:\n{system_prompt}\n\
             --------------------------------------------------------------------------------\n\
             USER PROMPT:\n{user_prompt}\n\
             --------------------------------------------------------------------------------\n\
             RESPONSE:\n{resp_body}\n\
             ================================================================================\n"
        );
    }
}

impl FacilitatorProvider for OllamaFacilitatorProvider {
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
        let client = self.client.clone();
        let base_url = resolve_ollama_url(&self.base_url);
        let url = format!("{base_url}/api/chat");
        let model = if self.model.trim().is_empty() {
            "qwen3:8b".to_string()
        } else {
            self.model.trim().to_string()
        };

        Box::pin(async move {
            let start = std::time::Instant::now();
            let user_prompt = build_facilitator_user_prompt(&context);
            let messages = vec![
                serde_json::json!({
                    "role": "system",
                    "content": context.system_prompt,
                }),
                serde_json::json!({
                    "role": "user",
                    "content": user_prompt,
                }),
            ];

            let body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "think": false,
                "options": {
                    "temperature": 0.85,
                    "repeat_penalty": 1.2,
                    "top_p": 0.9,
                }
            });

            let resp = client.post(&url).json(&body).send().await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    let err_msg = format!("HTTP request to Ollama at {url} failed: {e}");
                    log_llm_exchange(
                        &model,
                        context.is_inspiration,
                        context.focus_topic.as_deref(),
                        start.elapsed(),
                        &context.system_prompt,
                        &user_prompt,
                        Err(&err_msg),
                    );
                    return Err(err_msg);
                }
            };

            if !resp.status().is_success() {
                let err_msg = format!("Ollama returned HTTP error status: {}", resp.status());
                log_llm_exchange(
                    &model,
                    context.is_inspiration,
                    context.focus_topic.as_deref(),
                    start.elapsed(),
                    &context.system_prompt,
                    &user_prompt,
                    Err(&err_msg),
                );
                return Err(err_msg);
            }

            let json: Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    let err_msg = format!("Failed to parse Ollama JSON response: {e}");
                    log_llm_exchange(
                        &model,
                        context.is_inspiration,
                        context.focus_topic.as_deref(),
                        start.elapsed(),
                        &context.system_prompt,
                        &user_prompt,
                        Err(&err_msg),
                    );
                    return Err(err_msg);
                }
            };

            let text = json
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str());

            let text = match text {
                Some(t) if !t.trim().is_empty() => t.trim(),
                _ => {
                    let err_msg = "Ollama response missing or empty message.content".to_string();
                    log_llm_exchange(
                        &model,
                        context.is_inspiration,
                        context.focus_topic.as_deref(),
                        start.elapsed(),
                        &context.system_prompt,
                        &user_prompt,
                        Err(&err_msg),
                    );
                    return Err(err_msg);
                }
            };

            log_llm_exchange(
                &model,
                context.is_inspiration,
                context.focus_topic.as_deref(),
                start.elapsed(),
                &context.system_prompt,
                &user_prompt,
                Ok(text),
            );

            Ok(text.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_ollama_url() {
        assert_eq!(
            resolve_ollama_url("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            resolve_ollama_url("http://localhost:11434/"),
            "http://localhost:11434"
        );
        assert_eq!(
            resolve_ollama_url("localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            resolve_ollama_url("https://ollama.internal:11434/"),
            "https://ollama.internal:11434"
        );
        // Empty or whitespace falls back to resolvable host or localhost
        let empty_resolved = resolve_ollama_url("");
        assert!(
            empty_resolved == "http://ollama:11434" || empty_resolved == "http://localhost:11434"
        );
        let space_resolved = resolve_ollama_url("   ");
        assert_eq!(space_resolved, empty_resolved);
    }

    #[test]
    fn test_ollama_provider_normalizes_empty_url() {
        let provider = OllamaFacilitatorProvider::new("".to_string(), "".to_string());
        assert_eq!(provider.model, "qwen3:8b");
        assert!(provider.base_url.starts_with("http://"));
    }

    #[test]
    fn test_extract_proposal_items_and_structured_prompt() {
        let raw_inspiration = "**[Gap: Failure Recovery] Reconnect Window**: Allow 60s to rejoin.\n\n**[Elaborates t1] Score Tie-Breaker**: Resolve ties by sudden death.";
        let items = extract_proposal_items(raw_inspiration, true);
        assert_eq!(
            items,
            vec![
                "[Gap: Failure Recovery] Reconnect Window".to_string(),
                "[Elaborates t1] Score Tie-Breaker".to_string()
            ]
        );

        let ctx = FacilitatorContext {
            system_prompt: "sys".to_string(),
            session_title: "Multiplayer Chess".to_string(),
            is_inspiration: true,
            focus_topic: None,
            gap_questions: vec!["How are ratings updated?".to_string()],
            explored_items: items,
            turns: vec![(
                "stakeholder".to_string(),
                "Players should be matched by skill.".to_string(),
            )],
        };
        let prompt = build_facilitator_user_prompt(&ctx);
        assert!(prompt.contains("Session Goal / Title: Multiplayer Chess"));
        assert!(prompt.contains("[t1] (stakeholder): Players should be matched by skill."));
        assert!(prompt.contains("How are ratings updated?"));
        assert!(prompt.contains("[Gap: Failure Recovery] Reconnect Window"));
        assert!(prompt.contains("Spot the most important gaps in coverage"));
    }
}
