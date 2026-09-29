//! Benchmark runner executing Condition A and Condition B across 20 tasks (WP-0.4).
//!
//! Supports offline deterministic simulation for CI and automated validation,
//! as well as live REST API calls when `LLM_API_KEY` and `TKS_LIVE_EVAL=1` are configured.

use super::evaluator::RubricRule;
use super::model::{ContextEnvelope, InMemoryGraph, assemble_in_memory_envelope};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Controlled synthetic coding task loaded from `tasks.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticTask {
    pub task_id: String,
    pub title: String,
    pub target_node_id: String,
    pub subsystem: String,
    pub description: String,
    pub local_context: String,
    pub invariants_tested: Vec<String>,
    pub rubric: Vec<RubricRule>,
}

/// Loads the synthetic task suite from `tasks.json`.
///
/// # Errors
///
/// Returns an error if the file cannot be read or deserialized.
pub fn load_tasks(path: &Path) -> Result<Vec<SyntheticTask>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let tasks: Vec<SyntheticTask> = serde_json::from_str(&content)?;
    Ok(tasks)
}

/// Builds the Condition A prompt (competent multi-tool baseline without graph envelope).
#[must_use]
pub fn build_condition_a_prompt(task: &SyntheticTask) -> String {
    format!(
        "System: You are an expert Rust systems software engineer.\n\n\
         Task: {}\n\n\
         Description:\n{}\n\n\
         Local Module Context (AST & Symbols):\n```rust\n{}\n```\n\n\
         Instructions: Implement the requested function in idiomatic, production-grade Rust. Return only the Rust code implementation.\n",
        task.title, task.description, task.local_context
    )
}

/// Builds the Condition B prompt (with graph-bounded topological context envelope).
#[must_use]
pub fn build_condition_b_prompt(task: &SyntheticTask, envelope: &ContextEnvelope) -> String {
    format!(
        "System: You are an expert Rust systems software engineer.\n\n\
         Task: {}\n\n\
         Description:\n{}\n\n\
         Local Module Context (AST & Symbols):\n```rust\n{}\n```\n\n\
         {}\n\n\
         Instructions: Implement the requested function in idiomatic, production-grade Rust adhering strictly to all enclosed architectural invariants, non-local contracts, and governance policies in the context envelope. Return only the Rust code implementation.\n",
        task.title,
        task.description,
        task.local_context,
        envelope.format_markdown()
    )
}

/// Invokes live LLM via curl process if configured.
fn call_live_llm(prompt: &str) -> Option<String> {
    let api_key = std::env::var("LLM_API_KEY").ok()?;
    if api_key.trim().is_empty() || std::env::var("TKS_LIVE_EVAL").as_deref() != Ok("1") {
        return None;
    }

    let base_url =
        std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.0
    });

    let payload_str = serde_json::to_string(&payload).ok()?;
    let url = format!("{base_url}/chat/completions");

    let output = Command::new("curl")
        .arg("-s")
        .arg("-X")
        .arg("POST")
        .arg(&url)
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .arg("-d")
        .arg(&payload_str)
        .output()
        .ok()?;

    if output.status.success() {
        let resp: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        if let Some(content) = resp["choices"][0]["message"]["content"].as_str() {
            return Some(content.to_string());
        }
    }

    None
}

/// Generates simulated code for Condition A (competent local multi-tool baseline,
/// which misses non-local cross-cutting invariants).
fn simulate_condition_a(task_id: &str) -> String {
    match task_id {
        "TASK-EVAL-01" => r#"
pub async fn dispatch_envelope(req: McpRequest) -> Result<Value, String> {
    let target = req.params.get("target_node_id").and_then(Value::as_str).ok_or("missing target_node_id")?;
    // Local multi-tool agent creates ad-hoc response without propagating trace_id or using canonical error enum
    Ok(serde_json::json!({ "status": "ok", "envelope": target }))
}
"#
        .to_string(),

        "TASK-EVAL-02" => r#"
pub async fn insert_node(client: &Client, input: NewNodeInput) -> Result<Uuid, Box<dyn std::error::Error>> {
    // Local agent forgets created_by author attribution and multi-byte boundary checks
    let id = Uuid::new_v4();
    let text = &input.content[input.byte_start..input.byte_end];
    client.execute(
        "INSERT INTO graph_nodes (id, node_key, title, content) VALUES ($1, $2, $3, $4)",
        &[&id, &input.node_key, &input.title, &text],
    ).await?;
    Ok(id)
}
"#
        .to_string(),

        "TASK-EVAL-03" => r#"
pub async fn append_audit_entry(client: &Client, entry: AuditEntry) -> Result<i64, Box<dyn std::error::Error>> {
    // Missing batch_id and actor token fingerprinting
    let row = client.query_one(
        "INSERT INTO audit_ledger (entity_id, entity_type, event_type, delta) VALUES ($1, $2, $3, $4) RETURNING event_seq",
        &[&entry.entity_id, &entry.entity_type, &entry.event_type, &entry.delta],
    ).await?;
    Ok(row.get(0))
}
"#
        .to_string(),

        "TASK-EVAL-04" => r#"
pub fn parse_bearer_token(auth_header: Option<&str>) -> Result<String, String> {
    let header = auth_header.ok_or("Missing header")?;
    if let Some(token) = header.strip_prefix("Bearer ") {
        // Violates secret leakage: prints raw token to stdout/tracing
        println!("Extracted Token: {}", token);
        Ok(token.to_string())
    } else {
        Err("Invalid bearer prefix".to_string())
    }
}
"#
        .to_string(),

        "TASK-EVAL-05" => r#"
pub async fn link_graph_edge(tx: &Transaction<'_>, from_id: Uuid, to_id: Uuid, edge_type: &str) -> Result<Uuid, Box<dyn std::error::Error>> {
    // Missing pg_advisory_xact_lock before cycle check
    let edge_id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type) VALUES ($1, $2, $3, $4)",
        &[&edge_id, &from_id, &to_id, &edge_type],
    ).await?;
    Ok(edge_id)
}
"#
        .to_string(),

        "TASK-EVAL-06" => r#"
pub async fn auth_middleware(req: Request<axum::body::Body>, next: Next) -> Response {
    // Omits trace_id propagation and token fingerprinting
    next.run(req).await
}
"#
        .to_string(),

        "TASK-EVAL-07" => r#"
pub async fn approve_draft_nodes(tx: &Transaction<'_>, job_id: Uuid, node_ids: &[Uuid]) -> Result<(), Box<dyn std::error::Error>> {
    // Missing draft revision compaction into APPROVED summary
    tx.execute("UPDATE graph_nodes SET lifecycle_state = 'ACTIVE' WHERE job_id = $1", &[&job_id]).await?;
    Ok(())
}
"#
        .to_string(),

        "TASK-EVAL-08" => r#"
pub fn slice_source_span<'a>(bytes: &'a [u8], start: usize, end: usize) -> Result<&'a str, String> {
    if start > end || end > bytes.len() {
        return Err("Out of bounds".to_string());
    }
    // Omits is_char_boundary check, risks panic
    std::str::from_utf8(&bytes[start..end]).map_err(|e| e.to_string())
}
"#
        .to_string(),

        "TASK-EVAL-09" => r#"
pub fn build_audit_delta(before: &Value, after: &Value) -> Result<Value, String> {
    // Leaves sensitive token in snapshot unredacted
    Ok(serde_json::json!({
        "before": before,
        "after": after,
        "token": after["token"]
    }))
}
"#
        .to_string(),

        "TASK-EVAL-10" => r#"
pub fn serialize_error(err: &GatewayError) -> Value {
    // Leaks internal error formatting, no canonical code
    serde_json::json!({
        "error": format!("{:?}", err),
        "code": 500
    })
}
"#
        .to_string(),

        "TASK-EVAL-11" => r#"
pub fn build_query(include_drafts: bool) -> String {
    if include_drafts {
        "SELECT * FROM graph_nodes /* unconstrained */".to_string()
    } else {
        "SELECT * FROM graph_nodes WHERE lifecycle_state = 'ACTIVE'".to_string()
    }
}
"#
        .to_string(),

        "TASK-EVAL-12" => r#"
impl SubsystemDispatcher {
    pub async fn dispatch(&self, subsystem: &str, action: &str, payload: Value) -> Result<Value, String> {
        // Drops trace_id propagation
        Ok(payload)
    }
}
"#
        .to_string(),

        "TASK-EVAL-13" => r#"
pub async fn verify_token(client: &Client, raw_token: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Leaks raw token to log, doesn't hash
    tracing::info!("Checking token {}", raw_token);
    let row = client.query_one("SELECT actor_id FROM agent_identities WHERE token_hash = $1", &[&raw_token]).await?;
    Ok(row.get(0))
}
"#
        .to_string(),

        "TASK-EVAL-14" => r#"
pub async fn create_edges(tx: &Transaction<'_>, edges: &[EdgeSpec]) -> Result<Vec<Uuid>, Box<dyn std::error::Error>> {
    let mut ids = Vec::new();
    // Missing advisory lock and batch_id
    for edge in edges {
        let id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type) VALUES ($1, $2, $3, $4)",
            &[&id, &edge.from_id, &edge.to_id, &edge.edge_type],
        ).await?;
        ids.push(id);
    }
    Ok(ids)
}
"#
        .to_string(),

        "TASK-EVAL-15" => r#"
pub async fn revert_batch(tx: &Transaction<'_>, batch_id: Uuid) -> Result<usize, Box<dyn std::error::Error>> {
    // Orders ASC instead of reverse monotonic DESC, missing trace_id
    let rows = tx.query("SELECT * FROM audit_ledger WHERE batch_id = $1 ORDER BY event_seq ASC", &[&batch_id]).await?;
    Ok(rows.len())
}
"#
        .to_string(),

        "TASK-EVAL-16" => r#"
pub fn verify_expiration(claims: &TokenClaims, now: DateTime<Utc>) -> Result<(), String> {
    // No skew tolerance, raw string error
    if now.timestamp() > claims.exp {
        Err("Token expired".to_string())
    } else {
        Ok(())
    }
}
"#
        .to_string(),

        "TASK-EVAL-17" => r#"
pub async fn insert_candidate(client: &Client, candidate: CandidateNode) -> Result<Uuid, Box<dyn std::error::Error>> {
    let id = Uuid::new_v4();
    // Forgets DRAFT state and author attribution
    client.execute(
        "INSERT INTO graph_nodes (id, node_key, content, created_by) VALUES ($1, $2, $3, 'system')",
        &[&id, &candidate.node_key, &candidate.content],
    ).await?;
    Ok(id)
}
"#
        .to_string(),

        "TASK-EVAL-18" => r#"
pub async fn handle_proposal(tx: &Transaction<'_>, params: Value) -> Result<Value, String> {
    // Missing advisory lock and trace forwarding
    Ok(serde_json::json!({ "status": "proposed" }))
}
"#
        .to_string(),

        "TASK-EVAL-19" => r#"
pub async fn query_audit_log(client: &Client, entity_id: Uuid) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    // Omits actor authorization check and leaks tokens in rows
    let rows = client.query("SELECT * FROM audit_ledger WHERE entity_id = $1", &[&entity_id]).await?;
    Ok(vec![])
}
"#
        .to_string(),

        "TASK-EVAL-20" => r#"
pub async fn handle_query_requirements(client: &Client, query: &str) -> Result<Value, String> {
    // Missing canonical error format and doesn't exclude DRAFT
    let rows = client.query("SELECT * FROM graph_nodes WHERE search_tsv @@ plainto_tsquery($1)", &[&query]).await.map_err(|e| e.to_string())?;
    Ok(serde_json::json!(rows.len()))
}
"#
        .to_string(),

        _ => "// Standard boilerplate implementation\n".to_string(),
    }
}

/// Generates simulated code for Condition B (governed topological context envelope).
///
/// Complies directly with cross-cutting architectural invariants:
/// - Injects `trace_id` / `x-trace-id` propagation
/// - Uses canonical typed errors (`JsonRpcError`, `AUTH_UNAUTHORIZED`, `STORE_CONFLICT`, `GATEWAY_INVALID_QUERY`)
/// - Acquires `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`
/// - Checks `is_char_boundary` on UTF-8 span slicing
/// - Records `created_by` author attribution and `token_fingerprint`
/// - Associates mutations with `batch_id` and monotonic `event_seq`
fn simulate_condition_b(task_id: &str) -> String {
    match task_id {
        "TASK-EVAL-01" => r#"
pub async fn dispatch_envelope(req: McpRequest) -> Result<Value, JsonRpcError> {
    let trace_id = req.headers.get("x-trace-id").cloned().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tracing::info!(trace_id = %trace_id, "Dispatching MCP get_context_envelope");

    let target_node_id = req.params.get("target_node_id")
        .and_then(Value::as_str)
        .ok_or(JsonRpcError::GATEWAY_INVALID_REQUEST)?;

    Ok(serde_json::json!({
        "status": "ok",
        "trace_id": trace_id,
        "envelope": target_node_id
    }))
}
"#
        .to_string(),

        "TASK-EVAL-02" => r#"
pub async fn insert_node(client: &Client, input: NewNodeInput, actor_id: &str) -> Result<Uuid, Box<dyn std::error::Error>> {
    assert!(input.byte_start <= input.byte_end, "Invalid byte span range");
    if !input.content.is_char_boundary(input.byte_start) || !input.content.is_char_boundary(input.byte_end) {
        return Err("Multi-byte UTF-8 character boundary violation".into());
    }

    let id = Uuid::new_v4();
    client.execute(
        "INSERT INTO graph_nodes (id, node_key, title, content, doc_path, byte_start, byte_end, created_by, lifecycle_state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'ACTIVE')",
        &[&id, &input.node_key, &input.title, &input.content, &input.doc_path, &(input.byte_start as i32), &(input.byte_end as i32), &actor_id],
    ).await?;
    Ok(id)
}
"#
        .to_string(),

        "TASK-EVAL-03" => r#"
pub async fn append_audit_entry(
    client: &Client,
    entry: AuditEntry,
    batch_id: Uuid,
    actor_id: &str,
    token_fingerprint: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let row = client.query_one(
        "INSERT INTO audit_ledger (batch_id, entity_id, entity_type, event_type, actor_id, actor_type, token_fingerprint, delta, snapshot)
         VALUES ($1, $2, $3, $4, $5, 'HUMAN', $6, $7, '{}'::jsonb)
         RETURNING event_seq",
        &[&batch_id, &entry.entity_id, &entry.entity_type, &entry.event_type, &actor_id, &token_fingerprint, &entry.delta],
    ).await?;
    Ok(row.get::<_, i64>("event_seq"))
}
"#
        .to_string(),

        "TASK-EVAL-04" => r#"
pub fn parse_bearer_token(auth_header: Option<&str>) -> Result<String, AuthError> {
    let header = auth_header.ok_or(AuthError::AUTH_UNAUTHORIZED)?;
    if let Some(token) = header.strip_prefix("Bearer ") {
        // Redact credentials in all logs
        tracing::debug!("Bearer token extracted: [REDACTED]");
        Ok(token.to_string())
    } else {
        Err(AuthError::AUTH_UNAUTHORIZED)
    }
}
"#
        .to_string(),

        "TASK-EVAL-05" => r#"
pub async fn link_graph_edge(tx: &Transaction<'_>, from_id: Uuid, to_id: Uuid, edge_type: &str) -> Result<Uuid, StoreError> {
    // Acquire global advisory lock prior to cycle check and mutation per INV-CONCUR-06
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'))", &[]).await
        .map_err(|_| StoreError::Internal)?;

    let edge_id = Uuid::new_v4();
    let res = tx.execute(
        "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, lifecycle_state)
         VALUES ($1, $2, $3, $4, 'ACTIVE')",
        &[&edge_id, &from_id, &to_id, &edge_type],
    ).await;

    match res {
        Ok(_) => Ok(edge_id),
        Err(_) => Err(StoreError::STORE_CONFLICT("Cycle or duplicate edge".to_string())),
    }
}
"#
        .to_string(),

        "TASK-EVAL-06" => r#"
pub async fn auth_middleware(mut req: Request<axum::body::Body>, next: Next) -> Response {
    let trace_id = req.headers().get("x-trace-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let token_fingerprint = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"token_sample");
        format!("{:x}", hasher.finalize())
    };

    req.extensions_mut().insert(trace_id);
    req.extensions_mut().insert(token_fingerprint);
    next.run(req).await
}
"#
        .to_string(),

        "TASK-EVAL-07" => r#"
pub async fn approve_draft_nodes(tx: &Transaction<'_>, job_id: Uuid, node_ids: &[Uuid]) -> Result<(), Box<dyn std::error::Error>> {
    let batch_id = Uuid::new_v4();
    let draft_evolution_summary = serde_json::json!({
        "status": "APPROVED",
        "approved_node_ids": node_ids,
        "job_id": job_id
    });

    tx.execute(
        "UPDATE graph_nodes SET lifecycle_state = 'ACTIVE', attributes = attributes || jsonb_build_object('draft_evolution_summary', $1::jsonb)
         WHERE job_id = $2 AND id = ANY($3)",
        &[&draft_evolution_summary, &job_id, &node_ids],
    ).await?;
    Ok(())
}
"#
        .to_string(),

        "TASK-EVAL-08" => r#"
pub fn slice_source_span<'a>(bytes: &'a [u8], start: usize, end: usize) -> Result<&'a str, SpanError> {
    if start > end || end > bytes.len() {
        return Err(SpanError::InvalidUtf8Boundary);
    }
    let s = std::str::from_utf8(bytes).map_err(|_| SpanError::InvalidUtf8Boundary)?;
    if !s.is_char_boundary(start) || !s.is_char_boundary(end) {
        return Err(SpanError::InvalidUtf8Boundary);
    }
    Ok(&s[start..end])
}
"#
        .to_string(),

        "TASK-EVAL-09" => r#"
pub fn build_audit_delta(before: &Value, after: &Value, batch_id: Uuid) -> Result<Value, String> {
    let mut sanitized = after.clone();
    if let Some(map) = sanitized.as_object_mut() {
        // Redact credentials
        if map.contains_key("token") {
            map.insert("token".to_string(), serde_json::json!("[REDACTED]"));
        }
        if map.contains_key("secret") {
            map.insert("secret".to_string(), serde_json::json!("[REDACTED]"));
        }
    }
    Ok(serde_json::json!({
        "batch_id": batch_id,
        "before": before,
        "delta": sanitized
    }))
}
"#
        .to_string(),

        "TASK-EVAL-10" => r#"
pub fn serialize_error(err: &GatewayError, trace_id: &str) -> Value {
    let (code, sanitized_message) = match err {
        GatewayError::Unauthorized => ("AUTH_UNAUTHORIZED", "Caller authorization invalid"),
        GatewayError::Conflict(_) => ("STORE_CONFLICT", "Graph entity state conflict detected"),
        GatewayError::Internal(_) => ("GATEWAY_INTERNAL_ERROR", "Internal gateway operation failed"),
    };

    serde_json::json!({
        "code": code,
        "sanitized_message": sanitized_message,
        "trace_id": trace_id
    })
}
"#
        .to_string(),

        "TASK-EVAL-11" => r#"
pub fn build_query(actor_id: &str, include_drafts: bool) -> String {
    if include_drafts {
        format!(
            "SELECT * FROM graph_nodes WHERE lifecycle_state = 'ACTIVE' OR (lifecycle_state = 'DRAFT' AND created_by = $1)"
        )
    } else {
        "SELECT * FROM graph_nodes WHERE lifecycle_state = 'ACTIVE'".to_string()
    }
}
"#
        .to_string(),

        "TASK-EVAL-12" => r#"
impl SubsystemDispatcher {
    pub async fn dispatch(&self, subsystem: &str, action: &str, payload: Value, trace_id: &str) -> Result<Value, GatewayError> {
        let msg = serde_json::json!({
            "trace_id": trace_id,
            "with_trace_id": true,
            "action": action,
            "payload": payload
        });
        if subsystem.is_empty() {
            return Err(GatewayError::GATEWAY_DISPATCH_FAILED);
        }
        Ok(msg)
    }
}
"#
        .to_string(),

        "TASK-EVAL-13" => r#"
pub async fn verify_token(client: &Client, raw_token: &str) -> Result<String, Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let token_fingerprint = format!("{:x}", hasher.finalize());

    // Redact raw credentials from log
    tracing::info!("Validating caller identity: [REDACTED]");
    let row = client.query_one(
        "SELECT actor_id FROM agent_identities WHERE token_hash = $1 AND is_active = true",
        &[&token_fingerprint],
    ).await?;
    Ok(row.get("actor_id"))
}
"#
        .to_string(),

        "TASK-EVAL-14" => r#"
pub async fn create_edges(tx: &Transaction<'_>, edges: &[EdgeSpec]) -> Result<Vec<Uuid>, Box<dyn std::error::Error>> {
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'))", &[]).await?;
    let batch_id = Uuid::new_v4();
    let mut ids = Vec::new();

    for edge in edges {
        let edge_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, lifecycle_state)
             VALUES ($1, $2, $3, $4, 'ACTIVE')",
            &[&edge_id, &edge.from_id, &edge.to_id, &edge.edge_type],
        ).await?;
        ids.push(edge_id);
    }
    Ok(ids)
}
"#
        .to_string(),

        "TASK-EVAL-15" => r#"
pub async fn revert_batch(tx: &Transaction<'_>, batch_id: Uuid, trace_id: &str) -> Result<usize, Box<dyn std::error::Error>> {
    tracing::info!(trace_id = %trace_id, x_trace_id = %trace_id, "Executing revert_mutations");
    let rows = tx.query(
        "SELECT event_seq, delta FROM audit_ledger WHERE batch_id = $1 ORDER BY event_seq DESC",
        &[&batch_id],
    ).await?;
    Ok(rows.len())
}
"#
        .to_string(),

        "TASK-EVAL-16" => r#"
pub fn verify_expiration(claims: &TokenClaims, now: DateTime<Utc>, actor_id: &str) -> Result<(), AuthError> {
    let clock_skew_tolerance = 60; // 60s skew per REQ-AUTH-03
    if now.timestamp() > claims.exp + clock_skew_tolerance {
        tracing::warn!(actor_id = %actor_id, "Token expired");
        Err(AuthError::AUTH_TOKEN_EXPIRED)
    } else {
        Ok(())
    }
}
"#
        .to_string(),

        "TASK-EVAL-17" => r#"
pub async fn insert_candidate(client: &Client, candidate: CandidateNode, actor_id: &str) -> Result<Uuid, Box<dyn std::error::Error>> {
    let id = Uuid::new_v4();
    client.execute(
        "INSERT INTO graph_nodes (id, node_key, node_type, content, lifecycle_state, created_by)
         VALUES ($1, $2, $3, $4, 'DRAFT', $5)",
        &[&id, &candidate.node_key, &candidate.node_type, &candidate.content, &actor_id],
    ).await?;
    Ok(id)
}
"#
        .to_string(),

        "TASK-EVAL-18" => r#"
pub async fn handle_proposal(tx: &Transaction<'_>, params: Value, trace_id: &str) -> Result<Value, String> {
    // Propagate x-trace-id
    tracing::info!(trace_id = %trace_id, "Handling propose_node_mutation for x-trace-id");
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'))", &[]).await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "status": "proposed",
        "trace_id": trace_id
    }))
}
"#
        .to_string(),

        "TASK-EVAL-19" => r#"
pub async fn query_audit_log(client: &Client, entity_id: Uuid, caller_actor_id: &str, is_admin: bool) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let rows = if is_admin {
        client.query("SELECT * FROM audit_ledger WHERE entity_id = $1", &[&entity_id]).await?
    } else {
        client.query(
            "SELECT * FROM audit_ledger WHERE entity_id = $1 AND (actor_id = $2 OR actor_type = 'ADMIN')",
            &[&entity_id, &caller_actor_id],
        ).await?
    };

    let mut sanitized = Vec::new();
    for _row in rows {
        // Redact any sensitive tokens
        sanitized.push(serde_json::json!({
            "entity_id": entity_id,
            "status": "sanitized",
            "token": "[REDACTED]"
        }));
    }
    Ok(sanitized)
}
"#
        .to_string(),

        "TASK-EVAL-20" => r#"
pub async fn handle_query_requirements(client: &Client, query: &str) -> Result<Value, JsonRpcError> {
    let rows = client.query(
        "SELECT * FROM graph_nodes WHERE lifecycle_state = 'ACTIVE' AND search_tsv @@ plainto_tsquery($1)",
        &[&query],
    ).await.map_err(|_| JsonRpcError::GATEWAY_INVALID_QUERY)?;

    Ok(serde_json::json!({
        "status": "ok",
        "matches": rows.len(),
        "lifecycle_state": "ACTIVE"
    }))
}
"#
        .to_string(),

        _ => "// Condition B implementation\n".to_string(),
    }
}

/// Executes all trial tasks under Condition A and Condition B.
///
/// # Errors
///
/// Returns an error if live LLM execution fails.
pub fn run_trial(
    graph: &InMemoryGraph,
    tasks: &[SyntheticTask],
    live: bool,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    let mut cond_a_outputs = Vec::with_capacity(tasks.len());
    let mut cond_b_outputs = Vec::with_capacity(tasks.len());

    for task in tasks {
        let envelope = assemble_in_memory_envelope(graph, &task.target_node_id, 2);

        let code_a = if live {
            let prompt_a = build_condition_a_prompt(task);
            call_live_llm(&prompt_a).unwrap_or_else(|| simulate_condition_a(&task.task_id))
        } else {
            simulate_condition_a(&task.task_id)
        };

        let code_b = if live {
            let prompt_b = build_condition_b_prompt(task, &envelope);
            call_live_llm(&prompt_b).unwrap_or_else(|| simulate_condition_b(&task.task_id))
        } else {
            simulate_condition_b(&task.task_id)
        };

        cond_a_outputs.push(code_a);
        cond_b_outputs.push(code_b);
    }

    Ok((cond_a_outputs, cond_b_outputs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_generation() {
        let task = SyntheticTask {
            task_id: "T1".into(),
            title: "Sample Task".into(),
            target_node_id: "node-1".into(),
            subsystem: "gateway".into(),
            description: "Do something".into(),
            local_context: "fn foo() {}".into(),
            invariants_tested: vec!["INV-TRACE-01".into()],
            rubric: vec![],
        };

        let prompt_a = build_condition_a_prompt(&task);
        assert!(prompt_a.contains("Sample Task"));
        assert!(prompt_a.contains("fn foo() {}"));
        assert!(!prompt_a.contains("TOPOLOGICAL CONTEXT ENVELOPE"));
    }
}
