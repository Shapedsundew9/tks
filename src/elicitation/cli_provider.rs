//! Extensible CLI Facilitator Provider and Dual-Role Session Subsystem.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::elicitation::facilitator::{
    FacilitatorContext, FacilitatorProposal, FacilitatorProvider, GenerationMetadata,
    build_facilitator_directive, build_facilitator_user_prompt, elapsed_ms, log_llm_exchange,
};

/// Supported CLI facilitator backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CliBackendKind {
    Agy,
}

/// Metadata describing an advertised model from a CLI tool.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CliModelInfo {
    pub id: String,
    pub label: String,
    pub tier_rank: u32,
}

/// Availability and model capabilities of an installed CLI tool.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CliToolStatus {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub status: String, // "green" | "amber" | "red"
    pub message: Option<String>,
    pub models: Vec<CliModelInfo>,
    pub default_model: Option<String>,
}

/// Result returned from one turn of a CLI role session.
#[derive(Debug, Clone)]
pub struct CliTurnResult {
    pub text: String,
    pub model: String,
    pub conversation_id: Option<String>,
    pub usage: Option<tierfall::Usage>,
}

/// Role of a persistent CLI subprocess within an elicitation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliRole {
    Socratic,
    Inspiration,
}

pub type SpawnSessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn CliRoleSession>, String>> + Send + 'a>>;

/// Trait abstraction for an authenticated external CLI tool adapter (e.g. `agy`).
pub trait CliToolAdapter: Send + Sync {
    fn kind(&self) -> CliBackendKind;
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn probe(&self) -> Pin<Box<dyn Future<Output = CliToolStatus> + Send + '_>>;
    fn spawn_session(&self, model: &str) -> SpawnSessionFuture<'_>;
}

/// Trait abstraction for an active persistent role session driven over stdio stream-json.
pub trait CliRoleSession: Send {
    fn conversation_id(&self) -> Option<&str>;
    fn send_turn(
        &mut self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<CliTurnResult, String>> + Send + '_>>;
    fn close(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Ranks advertised models from lowest capability/cost upward so the least expensive
/// capable model is chosen as the default.
#[must_use]
pub fn rank_cli_model(id: &str) -> u32 {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("gpt-oss-120b") {
        0
    } else if lower.contains("flash") && lower.ends_with("-low") {
        10
    } else if lower.contains("flash") && lower.ends_with("-medium") {
        11
    } else if lower.contains("flash") && lower.ends_with("-high") {
        12
    } else if lower.contains("flash") {
        15
    } else if lower.ends_with("-low") && (lower.contains("sonnet") || lower.contains("gemini")) {
        20
    } else if lower.ends_with("-low") {
        25
    } else if lower.ends_with("-medium") && (lower.contains("sonnet") || lower.contains("gemini")) {
        30
    } else if lower.ends_with("-medium") {
        35
    } else if lower.ends_with("-high") && (lower.contains("sonnet") || lower.contains("gemini")) {
        40
    } else if lower.ends_with("-high") {
        45
    } else {
        50
    }
}

fn strip_ansi_and_spinners(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;

    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else if matches!(c, '⠋' | '⠙' | '⠹' | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏')
        {
            // Spinner Braille patterns - ignore
        } else {
            result.push(c);
        }
    }
    result
}

/// Parses the output of `agy models`, stripping terminal codes and ranking the models.
#[must_use]
pub fn parse_agy_models_output(raw: &str) -> Vec<CliModelInfo> {
    let normalized = raw.replace('\r', "\n");
    let mut models = Vec::new();

    for line in normalized.lines() {
        let clean = strip_ansi_and_spinners(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("Fetching available models") {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(id) = parts.next() else { continue };

        // Model IDs must consist of ascii alphanumeric or '-' / '.' / '_'
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        {
            continue;
        }

        let label = parts.collect::<Vec<_>>().join(" ");
        let label = if label.is_empty() {
            id.to_string()
        } else {
            label
        };

        let tier_rank = rank_cli_model(id);
        models.push(CliModelInfo {
            id: id.to_string(),
            label,
            tier_rank,
        });
    }

    models.sort_by(|a, b| a.tier_rank.cmp(&b.tier_rank).then_with(|| a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id);
    models
}

fn parse_usage(v: &serde_json::Value) -> Option<tierfall::Usage> {
    if !v.is_object() {
        return None;
    }
    let input_tokens = v
        .get("input_tokens")
        .or_else(|| v.get("prompt_tokens"))
        .and_then(serde_json::Value::as_u64);
    let output_tokens = v
        .get("output_tokens")
        .or_else(|| v.get("completion_tokens"))
        .and_then(serde_json::Value::as_u64);
    let total_tokens = v
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| match (input_tokens, output_tokens) {
            (Some(i), Some(o)) => Some(i + o),
            (Some(i), None) => Some(i),
            (None, Some(o)) => Some(o),
            (None, None) => None,
        });

    Some(tierfall::Usage {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        extra: Default::default(),
    })
}

fn is_exact_prefix(prefix: &[(String, String)], full: &[(String, String)]) -> bool {
    if prefix.len() > full.len() {
        return false;
    }
    &full[..prefix.len()] == prefix
}

/// Assembles the prompt sent to a persistent CLI role session.
#[must_use]
pub fn build_cli_turn_prompt(
    context: &FacilitatorContext,
    is_first_turn: bool,
    synced_turns: &[(String, String)],
    synced_explored: &[String],
) -> String {
    if is_first_turn {
        format!(
            "Role Persona & Rules:\n{}\n\nIMPORTANT: Do NOT call or invoke any tools, file readers, shell commands, or subagents. You are acting strictly as a conversational assistant in an intent elicitation session. Respond directly with the requested output only.\n\n{}",
            context.system_prompt,
            build_facilitator_user_prompt(context)
        )
    } else {
        let mut sections = Vec::new();

        if is_exact_prefix(synced_turns, &context.turns) {
            let remaining = &context.turns[synced_turns.len()..];
            if remaining.is_empty() {
                sections.push("No new statements recorded since your last turn.".to_string());
            } else {
                let lines: Vec<String> = remaining
                    .iter()
                    .enumerate()
                    .map(|(i, (role, text))| {
                        let idx = synced_turns.len() + i + 1;
                        format!("[t{idx}] ({role}): {text}")
                    })
                    .collect();
                sections.push(format!(
                    "New Statements Recorded Since Your Last Turn:\n{}",
                    lines.join("\n")
                ));
            }
        } else {
            let lines: Vec<String> = context
                .turns
                .iter()
                .enumerate()
                .map(|(i, (role, text))| {
                    let idx = i + 1;
                    format!("[t{idx}] ({role}): {text}")
                })
                .collect();
            sections.push(format!(
                "Transcript Updated (turns were edited, inserted, or deleted; use this updated list):\n{}",
                lines.join("\n")
            ));
        }

        let new_explored: Vec<&String> = context
            .explored_items
            .iter()
            .filter(|item| {
                !synced_explored
                    .iter()
                    .any(|synced| synced.eq_ignore_ascii_case(item.trim()))
            })
            .collect();
        if !new_explored.is_empty() {
            let items_text = new_explored
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!(
                "Newly Explored / Excluded Ideas & Questions (DO NOT repeat, rephrase, or overlap with these or previously excluded items):\n{items_text}"
            ));
        }

        sections.push(build_facilitator_directive(context));
        sections.join("\n\n")
    }
}

/// CLI adapter implementation for Antigravity CLI (`agy`).
pub struct AgyCliAdapter {
    cache: Arc<RwLock<Option<(Instant, CliToolStatus)>>>,
}

impl Default for AgyCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgyCliAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
        }
    }

    fn resolve_binary(&self) -> String {
        std::env::var("TKS_AGY_BIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "agy".to_string())
    }

    async fn probe_live(&self) -> CliToolStatus {
        let bin = self.resolve_binary();
        let timeout_duration = Duration::from_secs(5);
        let res =
            tokio::time::timeout(timeout_duration, Command::new(&bin).arg("models").output()).await;

        match res {
            Err(_) => CliToolStatus {
                id: "agy".to_string(),
                label: "Antigravity CLI (agy)".to_string(),
                available: false,
                status: "red".to_string(),
                message: Some("CLI probe timed out".to_string()),
                models: Vec::new(),
                default_model: None,
            },
            Ok(Err(_)) => CliToolStatus {
                id: "agy".to_string(),
                label: "Antigravity CLI (agy)".to_string(),
                available: false,
                status: "red".to_string(),
                message: Some("CLI binary not found".to_string()),
                models: Vec::new(),
                default_model: None,
            },
            Ok(Ok(output)) => {
                if !output.status.success() {
                    return CliToolStatus {
                        id: "agy".to_string(),
                        label: "Antigravity CLI (agy)".to_string(),
                        available: false,
                        status: "amber".to_string(),
                        message: Some("CLI not authenticated or no models available".to_string()),
                        models: Vec::new(),
                        default_model: None,
                    };
                }

                let raw = String::from_utf8_lossy(&output.stdout);
                let models = parse_agy_models_output(&raw);
                if models.is_empty() {
                    CliToolStatus {
                        id: "agy".to_string(),
                        label: "Antigravity CLI (agy)".to_string(),
                        available: false,
                        status: "amber".to_string(),
                        message: Some("CLI not authenticated or no models available".to_string()),
                        models: Vec::new(),
                        default_model: None,
                    }
                } else {
                    let default_model = Some(models[0].id.clone());
                    CliToolStatus {
                        id: "agy".to_string(),
                        label: "Antigravity CLI (agy)".to_string(),
                        available: true,
                        status: "green".to_string(),
                        message: None,
                        default_model,
                        models,
                    }
                }
            }
        }
    }
}

impl CliToolAdapter for AgyCliAdapter {
    fn kind(&self) -> CliBackendKind {
        CliBackendKind::Agy
    }

    fn id(&self) -> &'static str {
        "agy"
    }

    fn label(&self) -> &'static str {
        "Antigravity CLI (agy)"
    }

    fn probe(&self) -> Pin<Box<dyn Future<Output = CliToolStatus> + Send + '_>> {
        Box::pin(async move {
            let now = Instant::now();
            {
                let cache_guard = self.cache.read().await;
                if let Some((cached_at, ref status)) = *cache_guard
                    && now.duration_since(cached_at) < Duration::from_secs(30)
                {
                    return status.clone();
                }
            }

            let status = self.probe_live().await;
            let mut cache_guard = self.cache.write().await;
            *cache_guard = Some((now, status.clone()));
            status
        })
    }

    fn spawn_session(&self, model: &str) -> SpawnSessionFuture<'_> {
        let bin = self.resolve_binary();
        let model = model.to_string();
        Box::pin(async move {
            let workdir = std::env::temp_dir().join("tks-cli-facilitator");
            tokio::fs::create_dir_all(&workdir)
                .await
                .map_err(|e| format!("Failed to create isolated tempdir {workdir:?}: {e}"))?;

            let mut child = Command::new(&bin)
                .args([
                    "--model",
                    &model,
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--disable-slash-commands",
                    "--print=",
                ])
                .current_dir(&workdir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Failed to spawn CLI process '{bin}': {e}"))?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| "Failed to capture stdin of CLI child process".to_string())?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| "Failed to capture stdout of CLI child process".to_string())?;
            let mut reader = BufReader::new(stdout).lines();

            let startup_timeout = Duration::from_secs(15);
            let conv_id = tokio::time::timeout(startup_timeout, async {
                while let Some(line) = reader
                    .next_line()
                    .await
                    .map_err(|e| format!("Error reading from CLI stdout: {e}"))?
                {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
                        && val.get("event").and_then(serde_json::Value::as_str) == Some("init")
                    {
                        let conv_id = val
                            .get("conversation_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);
                        return Ok(conv_id);
                    }
                }
                Err("CLI process exited before sending init event".to_string())
            })
            .await
            .map_err(|_| "CLI process initialization timed out after 15s".to_string())??;

            let session = AgyRoleSession {
                child,
                stdin,
                reader,
                conversation_id: conv_id,
                model,
            };
            Ok(Box::new(session) as Box<dyn CliRoleSession>)
        })
    }
}

/// Active session running an `agy` subprocess over stdio NDJSON.
pub struct AgyRoleSession {
    child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    conversation_id: Option<String>,
    model: String,
}

impl CliRoleSession for AgyRoleSession {
    fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    fn send_turn(
        &mut self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<CliTurnResult, String>> + Send + '_>> {
        let prompt = prompt.to_string();
        Box::pin(async move {
            let user_msg = serde_json::json!({
                "event": "user",
                "message": {
                    "role": "user",
                    "content": prompt,
                }
            });
            let mut line = serde_json::to_string(&user_msg).map_err(|e| e.to_string())?;
            line.push('\n');

            self.stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("Failed to write to CLI stdin: {e}"))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| format!("Failed to flush CLI stdin: {e}"))?;

            let turn_timeout = Duration::from_secs(90);
            let mut accumulated_delta = String::new();
            let mut step_usage: Option<tierfall::Usage> = None;
            let mut final_result: Option<serde_json::Value> = None;

            tokio::time::timeout(turn_timeout, async {
                while let Some(raw_line) = self
                    .reader
                    .next_line()
                    .await
                    .map_err(|e| format!("Error reading from CLI stdout: {e}"))?
                {
                    let trimmed = raw_line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let val: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let event_type = val
                        .get("event")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if event_type == "step_update" {
                        let step_update_val = val.get("step_update").unwrap_or(&val);
                        if step_update_val
                            .get("step_type")
                            .and_then(serde_json::Value::as_str)
                            == Some("agent_response")
                        {
                            if let Some(delta) = step_update_val
                                .get("text_delta")
                                .and_then(serde_json::Value::as_str)
                            {
                                accumulated_delta.push_str(delta);
                            }
                            if step_update_val
                                .get("state")
                                .and_then(serde_json::Value::as_str)
                                == Some("DONE")
                                && let Some(usage_obj) = step_update_val.get("usage")
                            {
                                step_usage = parse_usage(usage_obj);
                            }
                        }
                    } else if event_type == "result" {
                        final_result = Some(val.get("result").unwrap_or(&val).clone());
                        break;
                    }
                }
                Ok::<(), String>(())
            })
            .await
            .map_err(|_| "CLI response timed out after 90s".to_string())??;

            let result = final_result
                .ok_or_else(|| "CLI process stream ended without result event".to_string())?;
            let status = result
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("UNKNOWN");
            if status != "SUCCESS" {
                let error_msg = result
                    .get("error")
                    .map(|v| {
                        if let Some(s) = v.as_str() {
                            s.to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .or_else(|| {
                        result
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                    })
                    .unwrap_or_else(|| format!("CLI turn ended with status '{status}'"));
                return Err(error_msg);
            }

            let response_str = result
                .get("response")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let text = if !response_str.trim().is_empty() {
                response_str.trim().to_string()
            } else {
                accumulated_delta.trim().to_string()
            };

            let usage = step_usage.or_else(|| result.get("usage").and_then(parse_usage));

            Ok(CliTurnResult {
                text,
                model: self.model.clone(),
                conversation_id: self.conversation_id.clone(),
                usage,
            })
        })
    }

    fn close(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let _ = self.child.start_kill();
        })
    }
}

struct ManagedRoleSession {
    tool_id: String,
    model: String,
    session: Box<dyn CliRoleSession>,
    synced_turns: Vec<(String, String)>,
    synced_explored: Vec<String>,
}

type RoleSessionSlot = Arc<Mutex<Option<ManagedRoleSession>>>;
type RoleSessionMap = Arc<RwLock<HashMap<(Uuid, CliRole), RoleSessionSlot>>>;

/// Dual-role stateful facilitator provider managing persistent CLI subprocesses.
#[derive(Clone)]
pub struct CliFacilitatorProvider {
    adapters: Arc<HashMap<String, Arc<dyn CliToolAdapter>>>,
    sessions: RoleSessionMap,
    target: Arc<RwLock<(String, Option<String>)>>,
}

impl CliFacilitatorProvider {
    #[must_use]
    pub fn new(cli_tool: String, cli_model: Option<String>) -> Self {
        let mut adapters: HashMap<String, Arc<dyn CliToolAdapter>> = HashMap::new();
        let agy = Arc::new(AgyCliAdapter::new());
        adapters.insert("agy".to_string(), agy);

        Self {
            adapters: Arc::new(adapters),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            target: Arc::new(RwLock::new((cli_tool, cli_model))),
        }
    }

    /// Updates the target CLI tool and model for future turns.
    pub async fn set_target(&self, tool: String, model: Option<String>) {
        let mut guard = self.target.write().await;
        *guard = (tool, model);
    }

    /// Returns the currently configured target CLI tool and model.
    pub async fn get_target(&self) -> (String, Option<String>) {
        self.target.read().await.clone()
    }

    /// Queries probe status for a specific registered CLI tool.
    pub async fn get_tool_status(&self, tool_id: &str) -> Option<CliToolStatus> {
        let adapter = self.adapters.get(tool_id)?;
        Some(adapter.probe().await)
    }

    /// Queries probe status for all registered CLI tools.
    pub async fn get_all_tool_statuses(&self) -> Vec<CliToolStatus> {
        let mut out = Vec::new();
        for adapter in self.adapters.values() {
            out.push(adapter.probe().await);
        }
        out
    }

    /// Closes and drops all open CLI subprocesses associated with an elicitation session.
    pub async fn close_session(&self, elicitation_id: Uuid) {
        let socratic_slot = {
            let read = self.sessions.read().await;
            read.get(&(elicitation_id, CliRole::Socratic)).cloned()
        };
        if let Some(slot) = socratic_slot {
            let mut guard = slot.lock().await;
            if let Some(mut managed) = guard.take() {
                managed.session.close().await;
            }
        }

        let inspiration_slot = {
            let read = self.sessions.read().await;
            read.get(&(elicitation_id, CliRole::Inspiration)).cloned()
        };
        if let Some(slot) = inspiration_slot {
            let mut guard = slot.lock().await;
            if let Some(mut managed) = guard.take() {
                managed.session.close().await;
            }
        }

        let mut write = self.sessions.write().await;
        write.remove(&(elicitation_id, CliRole::Socratic));
        write.remove(&(elicitation_id, CliRole::Inspiration));
    }
}

impl FacilitatorProvider for CliFacilitatorProvider {
    fn propose(
        &self,
        context: FacilitatorContext,
    ) -> Pin<Box<dyn Future<Output = Result<FacilitatorProposal, String>> + Send>> {
        let this = self.clone();
        Box::pin(async move {
            let start = Instant::now();
            let role = if context.is_inspiration {
                CliRole::Inspiration
            } else {
                CliRole::Socratic
            };
            let session_id = context.elicitation_id.unwrap_or_else(Uuid::nil);
            let (target_tool, target_model_opt) = this.get_target().await;

            let adapter = this
                .adapters
                .get(&target_tool)
                .cloned()
                .ok_or_else(|| format!("Unknown CLI tool: {target_tool}"))?;

            let target_model = match target_model_opt {
                Some(ref m) if !m.trim().is_empty() => m.clone(),
                _ => {
                    let status = adapter.probe().await;
                    status
                        .default_model
                        .or_else(|| status.models.first().map(|m| m.id.clone()))
                        .unwrap_or_else(|| "gpt-oss-120b-medium".to_string())
                }
            };

            let slot = {
                let mut map = this.sessions.write().await;
                map.entry((session_id, role))
                    .or_insert_with(|| Arc::new(Mutex::new(None)))
                    .clone()
            };

            let mut guard = slot.lock().await;

            // Invalidate session if tool or model changed
            if let Some(ref m) = *guard
                && (m.tool_id != target_tool || m.model != target_model)
                && let Some(mut old) = guard.take()
            {
                old.session.close().await;
            }

            let (turn_result, prompt_sent) = if guard.is_none() {
                let prompt = build_cli_turn_prompt(&context, true, &[], &[]);
                let mut session = adapter.spawn_session(&target_model).await?;
                let turn_result = match session.send_turn(&prompt).await {
                    Ok(res) => res,
                    Err(err) => {
                        session.close().await;
                        log_llm_exchange(
                            &format!("{target_tool}/{target_model}"),
                            context.is_inspiration,
                            context.focus_topic.as_deref(),
                            start.elapsed(),
                            &context.system_prompt,
                            &prompt,
                            Err(&err),
                        );
                        return Err(err);
                    }
                };
                *guard = Some(ManagedRoleSession {
                    tool_id: target_tool.clone(),
                    model: target_model.clone(),
                    session,
                    synced_turns: context.turns.clone(),
                    synced_explored: context.explored_items.clone(),
                });
                (turn_result, prompt)
            } else {
                let managed = guard.as_mut().unwrap();
                let prompt = build_cli_turn_prompt(
                    &context,
                    false,
                    &managed.synced_turns,
                    &managed.synced_explored,
                );
                match managed.session.send_turn(&prompt).await {
                    Ok(turn_res) => {
                        managed.synced_turns = context.turns.clone();
                        managed.synced_explored = context.explored_items.clone();
                        (turn_res, prompt)
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "tks::llm",
                            tool = %target_tool,
                            model = %target_model,
                            error = %err,
                            "CLI session turn failed; respawning and reseeding full transcript"
                        );
                        managed.session.close().await;
                        *guard = None;

                        // Respawn fresh session with first turn seeding
                        let fresh_prompt = build_cli_turn_prompt(&context, true, &[], &[]);
                        let mut session = adapter.spawn_session(&target_model).await?;
                        let turn_res = match session.send_turn(&fresh_prompt).await {
                            Ok(res) => res,
                            Err(retry_err) => {
                                session.close().await;
                                log_llm_exchange(
                                    &format!("{target_tool}/{target_model}"),
                                    context.is_inspiration,
                                    context.focus_topic.as_deref(),
                                    start.elapsed(),
                                    &context.system_prompt,
                                    &fresh_prompt,
                                    Err(&retry_err),
                                );
                                return Err(retry_err);
                            }
                        };
                        *guard = Some(ManagedRoleSession {
                            tool_id: target_tool.clone(),
                            model: target_model.clone(),
                            session,
                            synced_turns: context.turns.clone(),
                            synced_explored: context.explored_items.clone(),
                        });
                        (turn_res, fresh_prompt)
                    }
                }
            };

            log_llm_exchange(
                &format!("{target_tool}/{}", turn_result.model),
                context.is_inspiration,
                context.focus_topic.as_deref(),
                start.elapsed(),
                &context.system_prompt,
                &prompt_sent,
                Ok(&turn_result.text),
            );

            let generation = GenerationMetadata {
                provider: format!("cli:{target_tool}"),
                account: turn_result.conversation_id,
                model: turn_result.model,
                attempts: 1,
                duration_ms: elapsed_ms(start),
                usage: turn_result.usage,
            };

            Ok(FacilitatorProposal {
                text: turn_result.text,
                generation,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rank_cli_model_capability_ordering() {
        assert_eq!(rank_cli_model("gpt-oss-120b-medium"), 0);
        assert_eq!(rank_cli_model("gemini-3.8-flash-low"), 10);
        assert_eq!(rank_cli_model("gemini-3.8-flash-medium"), 11);
        assert_eq!(rank_cli_model("gemini-3.8-flash-high"), 12);
        assert_eq!(rank_cli_model("gemini-2.5-flash"), 15);
        assert_eq!(rank_cli_model("claude-sonnet-5-5-low"), 20);
        assert_eq!(rank_cli_model("gemini-4-argon-low"), 20);
        assert_eq!(rank_cli_model("claude-opus-5-5-low"), 25);
        assert_eq!(rank_cli_model("claude-sonnet-5-5-medium"), 30);
        assert_eq!(rank_cli_model("gemini-4-argon-medium"), 30);
        assert_eq!(rank_cli_model("claude-opus-5-5-medium"), 35);
        assert_eq!(rank_cli_model("claude-sonnet-5-5-high"), 40);
        assert_eq!(rank_cli_model("gemini-4-argon-high"), 40);
        assert_eq!(rank_cli_model("claude-opus-5-5-high"), 45);
        assert_eq!(rank_cli_model("some-random-custom-model"), 50);

        // Verify sorting ranks gpt-oss-120b first and flash second
        let mut ids = vec![
            "claude-opus-5-5-high",
            "gemini-3.8-flash-low",
            "gpt-oss-120b-medium",
            "claude-sonnet-5-5-medium",
        ];
        ids.sort_by_key(|id| rank_cli_model(id));
        assert_eq!(
            ids,
            vec![
                "gpt-oss-120b-medium",
                "gemini-3.8-flash-low",
                "claude-sonnet-5-5-medium",
                "claude-opus-5-5-high",
            ]
        );
    }

    #[test]
    fn test_parse_agy_models_output() {
        let raw = "\
\u{1b}[32m⠋ Fetching available models...\u{1b}[0m\r\
\u{1b}[32m⠙ Fetching available models...\u{1b}[0m\r\
gemini-4-argon-medium     Gemini 4 Argon (Medium)
gemini-3.8-flash-low      Gemini 3.8 Flash (Low)
gpt-oss-120b-medium       GPT-OSS 120B (Medium)
claude-opus-5-5-high      Claude Opus 5.5 (High)
";
        let parsed = parse_agy_models_output(raw);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].id, "gpt-oss-120b-medium");
        assert_eq!(parsed[0].tier_rank, 0);
        assert_eq!(parsed[1].id, "gemini-3.8-flash-low");
        assert_eq!(parsed[1].tier_rank, 10);
        assert_eq!(parsed[2].id, "gemini-4-argon-medium");
        assert_eq!(parsed[2].tier_rank, 30);
        assert_eq!(parsed[3].id, "claude-opus-5-5-high");
        assert_eq!(parsed[3].tier_rank, 45);
    }

    #[test]
    fn test_build_cli_turn_prompt_first_turn_and_incremental_turns() {
        let mut ctx = FacilitatorContext {
            elicitation_id: Some(Uuid::new_v4()),
            system_prompt: "You are the Socratic Facilitator.".to_string(),
            session_title: "Inventory App".to_string(),
            is_inspiration: false,
            focus_topic: None,
            gap_questions: Vec::new(),
            explored_items: vec!["Old Idea".to_string()],
            turns: vec![
                ("facilitator".to_string(), "What is your goal?".to_string()),
                ("stakeholder".to_string(), "Track items.".to_string()),
            ],
        };

        // First turn contains persona rules, system prompt, and full user prompt
        let prompt_first = build_cli_turn_prompt(&ctx, true, &[], &[]);
        assert!(prompt_first.contains("Role Persona & Rules:"));
        assert!(prompt_first.contains("You are the Socratic Facilitator."));
        assert!(prompt_first.contains("[t1] (facilitator): What is your goal?"));
        assert!(prompt_first.contains("[t2] (stakeholder): Track items."));

        // Second turn with tail append sends only new statements
        let synced_turns = ctx.turns.clone();
        let synced_explored = ctx.explored_items.clone();

        ctx.turns
            .push(("facilitator".to_string(), "Who has access?".to_string()));
        ctx.turns
            .push(("stakeholder".to_string(), "Admins only.".to_string()));
        ctx.explored_items.push("New Idea".to_string());

        let prompt_second = build_cli_turn_prompt(&ctx, false, &synced_turns, &synced_explored);
        assert!(!prompt_second.contains("Role Persona & Rules:"));
        assert!(!prompt_second.contains("[t1] (facilitator)"));
        assert!(prompt_second.contains("New Statements Recorded Since Your Last Turn:"));
        assert!(prompt_second.contains("[t3] (facilitator): Who has access?"));
        assert!(prompt_second.contains("[t4] (stakeholder): Admins only."));
        assert!(prompt_second.contains("Newly Explored / Excluded Ideas & Questions"));
        assert!(prompt_second.contains("- New Idea"));

        // Third turn with turn 1 edited triggers Transcript Updated resync
        let mut edited_ctx = ctx.clone();
        edited_ctx.turns[0] = (
            "facilitator".to_string(),
            "Edited Opening Question?".to_string(),
        );

        let prompt_edited =
            build_cli_turn_prompt(&edited_ctx, false, &synced_turns, &synced_explored);
        assert!(prompt_edited.contains(
            "Transcript Updated (turns were edited, inserted, or deleted; use this updated list):"
        ));
        assert!(prompt_edited.contains("[t1] (facilitator): Edited Opening Question?"));
    }
}
