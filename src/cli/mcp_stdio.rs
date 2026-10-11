//! Stdio streaming JSON-RPC proxy for Model Context Protocol (MCP) clients (WP-1.6, TB-4, D-19).
//!
//! Provides connection resilience with exponential backoff retry up to 2 seconds,
//! structured initialization error formatting with remediation code `-32000` on unreachable daemon,
//! and complete diagnostic stream separation (logs to stderr, protocol strictly to stdout).

use std::time::{Duration, Instant};

use clap::Args;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Arguments for `tks mcp-stdio`.
#[derive(Debug, Args, Clone)]
pub struct McpStdioArgs {
    /// Server base URL for the TKS daemon (default: http://127.0.0.1:8080).
    #[arg(long)]
    pub server_url: Option<String>,

    /// Optional bearer token for authentication.
    #[arg(long)]
    pub auth_token: Option<String>,
}

/// Executes the stdio proxy loop connecting external MCP clients to `tks serve`.
///
/// # Errors
///
/// Returns an error only on unrecoverable I/O errors.
pub async fn run_mcp_stdio(args: McpStdioArgs) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    run_mcp_stdio_io(args, stdin, stdout).await
}

/// Executes the stdio proxy loop over custom async reader and writer streams.
pub async fn run_mcp_stdio_io<R, W>(
    args: McpStdioArgs,
    mut reader: R,
    mut writer: W,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let resolved_url = crate::cli::resolve_server_url(args.server_url.as_deref());
    let base_url = resolved_url.trim_end_matches('/');
    let auth_token = crate::cli::resolve_auth_token(args.auth_token.as_deref());

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Step 1: Resilient Connection Check with Exponential Backoff (TB-4)
    let start_time = Instant::now();
    let max_retry_duration = Duration::from_millis(2000);
    let mut retry_delay = Duration::from_millis(50);
    let mut is_online = false;

    eprintln!("[tks-mcp-stdio] Checking connectivity to TKS daemon at {base_url}...");

    while start_time.elapsed() < max_retry_duration {
        let health_url = format!("{base_url}/health");
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                is_online = true;
                eprintln!("[tks-mcp-stdio] Successfully connected to TKS daemon.");
                break;
            }
            Ok(resp) => {
                eprintln!(
                    "[tks-mcp-stdio] Daemon responded with non-success status: {}, retrying...",
                    resp.status()
                );
            }
            Err(e) => {
                eprintln!("[tks-mcp-stdio] Connection attempt failed: {e}, retrying...");
            }
        }

        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_millis(500));
    }

    // Step 2: Handle Unreachable Daemon with Structured Initialization Error (TB-4)
    if !is_online {
        eprintln!(
            "[tks-mcp-stdio] TKS daemon is not running after {}ms retry window.",
            start_time.elapsed().as_millis()
        );
        eprintln!(
            "[tks-mcp-stdio] Waiting for incoming JSON-RPC on stdin to return remediation error..."
        );

        let mut line = String::new();
        // Read incoming request with a timeout so non-interactive callers or tests never lock up
        let read_result =
            tokio::time::timeout(Duration::from_millis(1000), reader.read_line(&mut line)).await;

        let req_id = match read_result {
            Ok(Ok(n)) if n > 0 => {
                let trimmed = line.trim();
                let req_json: Result<Value, _> = serde_json::from_str(trimmed);
                match &req_json {
                    Ok(val) => val.get("id").cloned().unwrap_or(Value::from(1)),
                    Err(_) => Value::from(1),
                }
            }
            _ => Value::from(1),
        };

        let err_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32000,
                "message": "TKS daemon is not running. Please start 'tks serve' in another terminal to enable Knowledge Substrate integration."
            }
        });

        let mut out_bytes = serde_json::to_vec(&err_response)?;
        out_bytes.push(b'\n');
        writer.write_all(&out_bytes).await?;
        writer.flush().await?;

        return Ok(());
    }

    // Step 3: Daemon is online; establish MCP SSE session
    let sse_url = format!("{base_url}/mcp/sse");
    let mut message_endpoint = format!("{base_url}/mcp/message");

    eprintln!("[tks-mcp-stdio] Establishing SSE session at {sse_url}...");
    match client
        .get(&sse_url)
        .header(AUTHORIZATION, format!("Bearer {auth_token}"))
        .send()
        .await
    {
        Ok(sse_resp) if sse_resp.status().is_success() => {
            let mut stream = sse_resp.bytes_stream();
            // Wait for initial endpoint event
            tokio::select! {
                chunk_opt = stream.next() => {
                    if let Some(Ok(bytes)) = chunk_opt {
                        let text = String::from_utf8_lossy(&bytes);
                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data:") {
                                let endpoint = data.trim();
                                if endpoint.starts_with('/') {
                                    message_endpoint = format!("{base_url}{endpoint}");
                                    eprintln!("[tks-mcp-stdio] Acquired message endpoint: {message_endpoint}");
                                    break;
                                }
                            }
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    eprintln!("[tks-mcp-stdio] SSE initial endpoint timeout; defaulting to {message_endpoint}");
                }
            }
        }
        Ok(resp) => {
            eprintln!(
                "[tks-mcp-stdio] SSE connection returned status {}; falling back to direct JSON-RPC",
                resp.status()
            );
            message_endpoint = format!("{base_url}/mcp");
        }
        Err(e) => {
            eprintln!(
                "[tks-mcp-stdio] SSE connection failed: {e}; falling back to direct JSON-RPC"
            );
            message_endpoint = format!("{base_url}/mcp");
        }
    }

    // Step 4: Stdio Streaming Loop
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            // EOF reached
            eprintln!("[tks-mcp-stdio] Stdin EOF received, terminating proxy.");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Validate JSON-RPC syntax
        let json_val: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {e}")
                    }
                });
                let mut out_bytes = serde_json::to_vec(&err_resp)?;
                out_bytes.push(b'\n');
                writer.write_all(&out_bytes).await?;
                writer.flush().await?;
                continue;
            }
        };

        // Forward request to daemon
        let resp_res = client
            .post(&message_endpoint)
            .header(AUTHORIZATION, format!("Bearer {auth_token}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&json_val)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::NO_CONTENT {
                    // JSON-RPC Notification: emit nothing to stdout
                    continue;
                }

                if resp.status().is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    if !body_text.trim().is_empty() {
                        let mut out_bytes = body_text.into_bytes();
                        if !out_bytes.ends_with(b"\n") {
                            out_bytes.push(b'\n');
                        }
                        writer.write_all(&out_bytes).await?;
                        writer.flush().await?;
                    }
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    let req_id = json_val.get("id").cloned().unwrap_or(Value::from(1));
                    let err_resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32603,
                            "message": format!("Internal daemon error (HTTP {status}): {err_text}")
                        }
                    });
                    let mut out_bytes = serde_json::to_vec(&err_resp)?;
                    out_bytes.push(b'\n');
                    writer.write_all(&out_bytes).await?;
                    writer.flush().await?;
                }
            }
            Err(e) => {
                let req_id = json_val.get("id").cloned().unwrap_or(Value::from(1));
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {
                        "code": -32000,
                        "message": format!("Failed to dispatch request to daemon: {e}")
                    }
                });
                let mut out_bytes = serde_json::to_vec(&err_resp)?;
                out_bytes.push(b'\n');
                writer.write_all(&out_bytes).await?;
                writer.flush().await?;
            }
        }
    }

    Ok(())
}
