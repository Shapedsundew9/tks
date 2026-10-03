//! Real-Time Substrate Web Explorer CLI client (WP-3.5, PHASE3-001, D-14, D-53).

use std::time::Duration;

use clap::{Args, Subcommand};
use reqwest::header::AUTHORIZATION;
use serde::Serialize;

/// Subcommands for the Substrate Web Explorer.
#[derive(Debug, Subcommand)]
pub enum ExplorerSubcommand {
    /// Launch or display connection details for the interactive Substrate Web Explorer.
    Serve(ExplorerServeArgs),
}

/// Arguments for `tks explorer serve [--port <port>] [--open] [--json]`.
#[derive(Debug, Args, Clone)]
pub struct ExplorerServeArgs {
    /// Port of the Substrate daemon server (default: 8080).
    #[arg(long, default_value = "8080")]
    pub port: u16,

    /// Host of the Substrate daemon server (default: 127.0.0.1).
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Automatically attempt to open the Substrate Web Explorer in a web browser.
    #[arg(long)]
    pub open: bool,

    /// Output explorer connection details in JSON format.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct ExplorerStatusResponse {
    pub url: String,
    pub api_endpoint: String,
    pub sse_endpoint: String,
    pub host: String,
    pub port: u16,
    pub server_status: String,
}

/// Executes the `tks explorer` CLI subcommand.
///
/// # Errors
///
/// Returns an error if argument parsing or JSON serialization fails.
pub async fn run_explorer(
    subcmd: ExplorerSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match subcmd {
        ExplorerSubcommand::Serve(args) => {
            // Determine effective base URL from server_url or arguments
            let base_url = if server_url.contains(&format!(":{}", args.port)) {
                server_url.trim_end_matches('/').to_string()
            } else {
                format!("http://{}:{}", args.host, args.port)
            };

            let explorer_url = format!("{base_url}/explorer");
            let api_endpoint = format!("{base_url}/api/v1/explorer/graph");
            let sse_endpoint = format!("{base_url}/api/v1/explorer/events");

            // Probe server connectivity with short timeout
            let client = reqwest::Client::new();
            let check_resp = client
                .get(&api_endpoint)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .timeout(Duration::from_millis(800))
                .send()
                .await;

            let server_status = match check_resp {
                Ok(resp) if resp.status().is_success() => "ONLINE",
                Ok(resp) => {
                    tracing::debug!("Explorer probe returned status: {}", resp.status());
                    "ONLINE"
                }
                Err(e) => {
                    tracing::debug!("Server probe error: {e}");
                    "OFFLINE"
                }
            };

            // Optionally open browser if requested
            if args.open {
                open_browser(&explorer_url);
            }

            if args.json {
                let status_payload = ExplorerStatusResponse {
                    url: explorer_url,
                    api_endpoint,
                    sse_endpoint,
                    host: args.host,
                    port: args.port,
                    server_status: server_status.to_string(),
                };
                println!("{}", serde_json::to_string_pretty(&status_payload)?);
            } else {
                println!("Substrate Web Explorer:");
                println!("  URL:           {explorer_url}");
                println!("  API Endpoint:  {api_endpoint}");
                println!("  SSE Stream:    {sse_endpoint}");
                println!("  Server Status: {server_status}");
                if server_status == "OFFLINE" {
                    println!(
                        "\n  Note: The TKS daemon is not running on port {}.",
                        args.port
                    );
                    println!("        Start it with: `tks serve --port {}`", args.port);
                }
            }
        }
    }

    Ok(())
}

/// Attempts to open a URL in the default system browser in a cross-platform manner.
fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}
