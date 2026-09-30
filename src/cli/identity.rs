//! Identity management CLI client (WP-1.6, TB-5, D-49).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

/// CLI subcommands for identity administration.
#[derive(Debug, Subcommand)]
pub enum IdentitySubcommand {
    /// Provision a new human or agent identity.
    Create(CreateIdentityArgs),
    /// Revoke an existing agent identity and invalidate its cache immediately.
    Revoke(RevokeIdentityArgs),
}

/// Arguments for `tks identity create`.
#[derive(Debug, Args)]
pub struct CreateIdentityArgs {
    /// Human-readable agent or supervisor name.
    #[arg(long)]
    pub name: String,

    /// Actor role (`HUMAN` or `AGENT`).
    #[arg(long)]
    pub role: String,

    /// Optional pre-shared bearer token.
    #[arg(long)]
    pub token: Option<String>,
}

/// Arguments for `tks identity revoke`.
#[derive(Debug, Args)]
pub struct RevokeIdentityArgs {
    /// Canonical identity identifier to revoke.
    pub agent_id: String,
}

#[derive(Serialize)]
struct CreateIdentityPayload<'a> {
    name: &'a str,
    role: &'a str,
    token: Option<&'a str>,
}

#[derive(Deserialize)]
struct CreateIdentityResult {
    agent_id: String,
    token: String,
}

#[derive(Deserialize)]
struct RevokeIdentityResult {
    status: String,
}

/// Executes the `tks identity` CLI subcommand.
///
/// # Errors
///
/// Returns an error if HTTP request fails or server returns an error.
pub async fn run_identity(
    subcmd: IdentitySubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        IdentitySubcommand::Create(args) => {
            let url = format!("{base_url}/api/v1/identities");
            let payload = CreateIdentityPayload {
                name: &args.name,
                role: &args.role,
                token: args.token.as_deref(),
            };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to create identity ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: CreateIdentityResult = resp.json().await?;
            println!("Identity created successfully:");
            println!("  agent_id: {}", result.agent_id);
            println!("  token:    {}", result.token);
        }

        IdentitySubcommand::Revoke(args) => {
            let url = format!("{base_url}/api/v1/identities/{}/revoke", args.agent_id);

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to revoke identity ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: RevokeIdentityResult = resp.json().await?;
            println!(
                "Identity '{}' revoked successfully (status: {}).",
                args.agent_id, result.status
            );
        }
    }

    Ok(())
}
