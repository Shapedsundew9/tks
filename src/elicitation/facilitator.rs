//! Socratic facilitator providers using direct Ollama or Tierfall routing.

use serde_json::Value;
use std::time::Duration;

/// Facilitator conversation context assembled for proposal generation.
#[derive(Debug, Clone, Default)]
pub struct FacilitatorContext {
    pub elicitation_id: Option<uuid::Uuid>,
    pub system_prompt: String,
    pub session_title: String,
    pub is_inspiration: bool,
    pub focus_topic: Option<String>,
    pub gap_questions: Vec<String>,
    pub explored_items: Vec<String>,
    pub turns: Vec<(String, String)>, // (role, text)
}

/// Operational metadata, without prompts or credentials.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GenerationMetadata {
    pub provider: String,
    pub account: Option<String>,
    pub model: String,
    pub attempts: u32,
    pub duration_ms: u64,
    pub usage: Option<tierfall::Usage>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FacilitatorProposal {
    pub text: String,
    pub generation: GenerationMetadata,
}

pub(crate) fn elapsed_ms(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Shares discovery, cooldowns, and quota state across all proposals.
#[derive(Debug, Clone)]
pub struct TierfallFacilitatorProvider {
    pub client: tierfall::Tierfall,
    pub route: String,
}

impl FacilitatorProvider for TierfallFacilitatorProvider {
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FacilitatorProposal, String>> + Send>,
    > {
        let client = self.client.clone();
        let route = self.route.clone();
        Box::pin(async move {
            let start = std::time::Instant::now();
            let prompt = build_facilitator_user_prompt(&context);
            let request = tierfall::ChatRequest::new(&route)
                .system(context.system_prompt.clone())
                .user(prompt.clone());
            // Upstream error bodies can contain echoed prompts; keep them out of logs and SSE.
            let reply = client.chat(request).await.map_err(|error| {
                let message = match error {
                    tierfall::Error::AllExhausted { .. } | tierfall::Error::Upstream { .. } => format!("Tierfall: {error}"),
                    tierfall::Error::Json(_) => "Tierfall returned invalid completion JSON".to_string(),
                    tierfall::Error::Config(_) | tierfall::Error::InvalidRequest(_) |
                    tierfall::Error::UnknownModel(_) => format!("Tierfall: {error}"),
                    tierfall::Error::Http(error) => format!("Tierfall HTTP error: {}", error.without_url()),
                    tierfall::Error::Io(error) => format!("Tierfall I/O error: {}", error.kind()),
                    tierfall::Error::Stream(_) => "Tierfall returned an unexpected streaming error".to_string(),
                };
                tracing::warn!(target: "tks::llm", route, duration_ms = elapsed_ms(start), "{message}");
                log_llm_exchange(&route, context.is_inspiration, context.focus_topic.as_deref(),
                    start.elapsed(), &context.system_prompt, &prompt, Err(&message));
                message
            })?;
            let text = reply
                .text()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    tracing::warn!(target: "tks::llm", route, "Tierfall response missing or empty assistant text");
                    "Tierfall response missing or empty assistant text".to_string()
                })?;
            let served = reply
                .served_by
                .ok_or_else(|| "Tierfall response missing serving metadata".to_string())?;
            log_llm_exchange(
                &format!("{}/{}", served.provider, served.model),
                context.is_inspiration,
                context.focus_topic.as_deref(),
                start.elapsed(),
                &context.system_prompt,
                &prompt,
                Ok(&*text),
            );
            let generation = GenerationMetadata {
                provider: served.provider.to_string(),
                account: Some(served.account),
                model: served.model,
                attempts: served.attempts,
                duration_ms: elapsed_ms(start),
                usage: reply.usage,
            };
            tracing::info!(target: "tks::llm", route, provider = generation.provider,
                model = generation.model, attempts = generation.attempts,
                duration_ms = generation.duration_ms, "Facilitator proposal completed");
            Ok(FacilitatorProposal {
                text: text.trim().to_string(),
                generation,
            })
        })
    }
}

/// Maximum number of inspiration ideas surfaced per proposal (prompt contract: 1 to 3).
pub const MAX_INSPIRATION_IDEAS: usize = 3;

/// One structured Inspiration idea parsed from an LLM response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InspirationIdea {
    /// Gap / axis category such as `Exception Policy` (may be empty).
    #[serde(default, alias = "gap", alias = "axis", alias = "tag")]
    pub category: String,
    pub title: String,
    #[serde(
        alias = "body",
        alias = "requirement",
        alias = "description",
        alias = "text"
    )]
    pub statement: String,
}

impl InspirationIdea {
    /// Label used for explored-item memory, e.g. `[Exception Policy] Reconnect Window`.
    #[must_use]
    pub fn label(&self) -> String {
        if self.category.is_empty() {
            self.title.clone()
        } else {
            format!("[{}] {}", self.category, self.title)
        }
    }

    fn normalized(self) -> Option<Self> {
        let category = clean_inline(&self.category);
        let category = category
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim()
            .to_string();
        let title = clean_inline(&self.title)
            .trim_end_matches(':')
            .trim()
            .to_string();
        let statement = clean_inline(&self.statement);
        (!title.is_empty() && !statement.is_empty()).then_some(Self {
            category,
            title,
            statement,
        })
    }
}

/// Strips inline markdown emphasis and collapses whitespace.
fn clean_inline(text: &str) -> String {
    text.replace("**", "")
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Removes `<think>…</think>` blocks emitted by reasoning models.
fn strip_think_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn parse_ideas_json(text: &str) -> Option<Vec<InspirationIdea>> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        ideas: Vec<InspirationIdea>,
    }

    let candidates = [
        text.find('{').zip(text.rfind('}')),
        text.find('[').zip(text.rfind(']')),
    ];
    for (start, end) in candidates.into_iter().flatten() {
        if end <= start {
            continue;
        }
        let slice = &text[start..=end];
        if let Ok(envelope) = serde_json::from_str::<Envelope>(slice) {
            return Some(envelope.ideas);
        }
        if let Ok(ideas) = serde_json::from_str::<Vec<InspirationIdea>>(slice) {
            return Some(ideas);
        }
    }
    None
}

/// Parses `**[Category] Title**: statement` lines, ignoring headings, tables, rules and prose.
fn parse_ideas_markdown(text: &str) -> Vec<InspirationIdea> {
    let mut ideas: Vec<InspirationIdea> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with('|')
            || line.starts_with("---")
            || line.starts_with("```")
        {
            open = false;
            continue;
        }
        // Drop list markers such as `1.`, `2)`, `-`, `•`, or `* ` before a bold title.
        let item = line.trim_start_matches(|c: char| {
            c.is_ascii_digit() || c.is_whitespace() || matches!(c, '.' | ')' | '-' | '•')
        });
        let item = item.strip_prefix("* ").unwrap_or(item).trim_start();
        if let Some(after) = item.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            let raw_title = after[..end].trim();
            let rest = after[end + 2..].trim_start();
            let (raw_title, statement) = if let Some(stripped) = raw_title.strip_suffix(':') {
                (stripped, rest)
            } else if let Some(stripped) = rest.strip_prefix(':') {
                (raw_title, stripped)
            } else {
                open = false;
                continue;
            };
            let (category, title) =
                match raw_title.strip_prefix('[').and_then(|r| r.split_once(']')) {
                    Some((cat, title)) => (cat.trim(), title.trim()),
                    None => ("", raw_title),
                };
            ideas.push(InspirationIdea {
                category: category.to_string(),
                title: title.to_string(),
                statement: statement.trim().to_string(),
            });
            open = true;
        } else if open && let Some(last) = ideas.last_mut() {
            last.statement.push(' ');
            last.statement.push_str(line);
        }
    }
    ideas
}

/// Parses structured Inspiration ideas from an LLM response.
///
/// Accepts the JSON contract of [`INSPIRATION_SYSTEM_PROMPT_V3`](super::INSPIRATION_SYSTEM_PROMPT_V3)
/// (optionally fenced or surrounded by prose), falling back to strict
/// `**[Category] Title**: statement` markdown lines. Analysis scaffolding such as
/// headings, tables, and horizontal rules is discarded. Returns at most
/// [`MAX_INSPIRATION_IDEAS`] ideas.
#[must_use]
pub fn parse_inspiration_ideas(text: &str) -> Vec<InspirationIdea> {
    let text = strip_think_blocks(text);
    let raw = parse_ideas_json(&text).unwrap_or_else(|| parse_ideas_markdown(&text));
    raw.into_iter()
        .filter_map(InspirationIdea::normalized)
        .take(MAX_INSPIRATION_IDEAS)
        .collect()
}

/// Renders ideas as canonical `**[Category] Title**: statement` paragraphs.
#[must_use]
pub fn render_inspiration_ideas(ideas: &[InspirationIdea]) -> String {
    ideas
        .iter()
        .map(|idea| format!("**{}**: {}", idea.label(), idea.statement))
        .collect::<Vec<_>>()
        .join("\n\n")
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
    parse_inspiration_ideas(trimmed)
        .iter()
        .map(InspirationIdea::label)
        .collect()
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

    sections.push(build_facilitator_directive(context));
    sections.join("\n\n")
}

/// Assembles the task directive section from the [`FacilitatorContext`].
#[must_use]
pub fn build_facilitator_directive(context: &FacilitatorContext) -> String {
    if context.is_inspiration {
        let body = if let Some(ref topic) = context.focus_topic {
            format!(
                "The stakeholder specifically wants to explore domain rules, invariants, and edge cases regarding: \"{topic}\".\n\
                 Using the established statements above as context, propose 1 to 3 concrete, non-overlapping domain requirement statements regarding \"{topic}\" that establish unexamined preconditions, exceptions, or authority boundaries not already stated or listed in the excluded items."
            )
        } else {
            "Silently check the 3 Foundation Anchors (real-world outcome, primary actors/roles, core operational scope/invariants).\n\
             - If ANY anchor is indeterminate or unstated, the ideas must establish the missing foundations (categories Outcome, Actor, or Scope).\n\
             - Otherwise, silently audit the 5 orthogonal domain axes (any axis addressed or explicitly declared Out of Scope counts as covered), spot the most important gaps in coverage on the axis with zero or lowest coverage, and propose ideas that nail down invariants, negative boundaries, or exception policies there.\n\
             Do NOT repeat or rehash what is already covered in the transcript or the excluded list."
                .to_string()
        };
        format!(
            "Task:\n{body}\n\n\
             Do NOT output your analysis, step labels, or tables. Respond with ONLY the JSON object {{\"ideas\":[{{\"category\":…,\"title\":…,\"statement\":…}}]}} defined in the system prompt."
        )
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
         Do NOT ask generic boilerplate questions or repeat anything from the Already Explored list.\n\
         \n\
         Perform this check silently: respond with ONLY the question itself — no step labels, anchor assessment, or commentary."
            .to_string()
    }
}

/// Provider trait for proposing facilitator Socratic questions.
pub trait FacilitatorProvider: Send + Sync {
    /// Proposes the next neutral Socratic question.
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FacilitatorProposal, String>> + Send>,
    >;
}

/// No-op provider when facilitator assistance is disabled (`none`).
#[derive(Debug, Clone, Default)]
pub struct NoneFacilitatorProvider;

impl FacilitatorProvider for NoneFacilitatorProvider {
    fn propose(
        &self,
        _context: FacilitatorContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FacilitatorProposal, String>> + Send>,
    > {
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

/// Queries the Ollama tags endpoint to determine reachability and available models.
pub async fn probe_ollama_status(
    client: &reqwest::Client,
    base_url: &str,
    configured_model: &str,
) -> serde_json::Value {
    let resolved = resolve_ollama_url(base_url);
    let url = format!("{resolved}/api/tags");
    let resp = tokio::time::timeout(Duration::from_millis(800), client.get(&url).send()).await;

    match resp {
        Ok(Ok(res)) if res.status().is_success() => {
            let body = res.json::<serde_json::Value>().await.unwrap_or_default();
            let models: Vec<String> = body
                .get("models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            m.get("name")
                                .or_else(|| m.get("model"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();

            if models.is_empty() {
                serde_json::json!({
                    "available": false,
                    "status": "amber",
                    "model": configured_model,
                    "models": models,
                    "label": "Ollama (Local LLM)"
                })
            } else {
                serde_json::json!({
                    "available": true,
                    "status": "green",
                    "model": configured_model,
                    "models": models,
                    "label": "Ollama (Local LLM)"
                })
            }
        }
        _ => serde_json::json!({
            "available": false,
            "status": "red",
            "model": configured_model,
            "models": Vec::<String>::new(),
            "label": "Ollama (Local LLM)"
        }),
    }
}

pub(crate) fn log_llm_exchange(
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
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FacilitatorProposal, String>> + Send>,
    > {
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

            let mut body = serde_json::json!({
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
            if context.is_inspiration {
                body["format"] = Value::from("json");
            }

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

            let prompt_tokens = json.get("prompt_eval_count").and_then(Value::as_u64);
            let completion_tokens = json.get("eval_count").and_then(Value::as_u64);
            Ok(FacilitatorProposal {
                text: text.to_string(),
                generation: GenerationMetadata {
                    provider: "ollama".to_string(),
                    account: None,
                    model: json
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or(&model)
                        .to_string(),
                    attempts: 1,
                    duration_ms: elapsed_ms(start),
                    usage: if prompt_tokens.is_some() || completion_tokens.is_some() {
                        Some(tierfall::Usage {
                            prompt_tokens,
                            completion_tokens,
                            ..Default::default()
                        })
                    } else {
                        None
                    },
                },
            })
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
            elicitation_id: None,
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
        assert!(prompt.contains("spot the most important gaps in coverage"));
        assert!(prompt.contains("Respond with ONLY the JSON object"));
        assert!(!prompt.contains("Step 1"));
    }

    #[test]
    fn test_parse_inspiration_ideas_json_contract() {
        let raw = r#"{"ideas":[{"category":"Exception Policy","title":"Illegal Cell Selection","statement":"An occupied cell cannot be selected; the turn stays with the same player."},{"category":"Lifecycle Rule","title":"Result Before Reset","statement":"The final result is announced before a new game can begin."}]}"#;
        let ideas = parse_inspiration_ideas(raw);
        assert_eq!(ideas.len(), 2);
        assert_eq!(ideas[0].category, "Exception Policy");
        assert_eq!(ideas[0].title, "Illegal Cell Selection");
        assert_eq!(
            ideas[0].label(),
            "[Exception Policy] Illegal Cell Selection"
        );
        assert_eq!(
            extract_proposal_items(raw, true),
            vec![
                "[Exception Policy] Illegal Cell Selection".to_string(),
                "[Lifecycle Rule] Result Before Reset".to_string()
            ]
        );
    }

    #[test]
    fn test_parse_inspiration_ideas_tolerates_wrappers_and_aliases() {
        let raw = "<think>{\"draft\": true}</think>Here you go:\n```json\n[{\"gap\":\"[Actor]\",\"title\":\"**Automated Opponent**:\",\"body\":\"The computer  picks its move\\nwithout human help.\"},{\"title\":\"\",\"statement\":\"dropped\"}]\n```";
        let ideas = parse_inspiration_ideas(raw);
        assert_eq!(
            ideas,
            vec![InspirationIdea {
                category: "Actor".to_string(),
                title: "Automated Opponent".to_string(),
                statement: "The computer picks its move without human help.".to_string(),
            }]
        );
    }

    #[test]
    fn test_parse_inspiration_ideas_drops_analysis_scaffolding() {
        // Shape of a real garbled response: step headings, tables, rules, and prose
        // interleaved with the actual ideas.
        let raw = "**Step 1 – Foundation Anchors Check**\n\n\
            | Anchor | Status | Gap |\n|---|---|---|\n| **Real-world outcome** | Missing | **[Outcome] Game Result**: must be shown. |\n\n\
            All three foundational anchors are now explicitly defined.\n\n---\n\n\
            ### Domain Requirement Statements\n\n\
            1. **[Accountability] Game Move Ledger**: After every turn, a sequential list of moves\n   is displayed.\n\n\
            - **[Exception Policy] Illegal Cell Selection Rejection**: An occupied cell is refused.\n\
            **Persistent Final Board:** The final board stays visible.\n\
            **[Constraint] Extra Idea**: Beyond the cap.";
        let ideas = parse_inspiration_ideas(raw);
        assert_eq!(ideas.len(), MAX_INSPIRATION_IDEAS);
        assert_eq!(ideas[0].label(), "[Accountability] Game Move Ledger");
        assert_eq!(
            ideas[0].statement,
            "After every turn, a sequential list of moves is displayed."
        );
        assert_eq!(ideas[1].category, "Exception Policy");
        assert_eq!(ideas[2].label(), "Persistent Final Board");
        assert_eq!(ideas[2].statement, "The final board stays visible.");
        assert!(parse_inspiration_ideas("**Step 1 – Check**\n\nJust prose.").is_empty());
        assert_eq!(
            render_inspiration_ideas(&ideas[..1]),
            "**[Accountability] Game Move Ledger**: After every turn, a sequential list of moves is displayed."
        );
    }
}
