//! TKS (The Knowledge Substrate) Library.

use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod cli;
pub mod db;
pub mod gateway;
pub mod ingest;
pub mod storage;
pub mod worker;

use cli::{
    AdminSubcommand, ExplorerSubcommand, IdentitySubcommand, IngestArgs, McpStdioArgs, ServeArgs,
    StagingSubcommand, TaskSubcommand, WorkspaceSubcommand, resolve_auth_token, resolve_server_url,
    run_admin, run_explorer, run_identity, run_ingest, run_mcp_stdio, run_serve, run_staging,
    run_task, run_workspace,
};

/// Command-line arguments for the TKS CLI.
#[derive(Parser, Debug)]
#[command(name = "tks", about = "The Knowledge Substrate", version = "0.1.0")]
pub struct Cli {
    /// Run pending database migrations against $DATABASE_URL and exit immediately.
    #[arg(long, global = true)]
    pub migrate_only: bool,

    /// Optional database connection URL (overrides $DATABASE_URL).
    #[arg(long, global = true)]
    pub database_url: Option<String>,

    /// Server base URL for daemon commands (default: http://127.0.0.1:8080).
    #[arg(long, global = true)]
    pub server_url: Option<String>,

    /// Bearer authentication token for daemon commands.
    #[arg(long, global = true)]
    pub auth_token: Option<String>,

    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available TKS subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the unified Knowledge Substrate daemon server.
    Serve(ServeArgs),

    /// Ingest a specification document into the Knowledge Substrate.
    Ingest(IngestArgs),

    /// Stdio streaming JSON-RPC proxy for MCP clients.
    #[command(name = "mcp-stdio")]
    McpStdio(McpStdioArgs),

    /// Candidate draft review and staging promotion/rejection.
    #[command(subcommand)]
    Staging(StagingSubcommand),

    /// Identity provisioning and revocation management.
    #[command(subcommand)]
    Identity(IdentitySubcommand),

    /// Execution task operations (create, update, list).
    #[command(subcommand)]
    Task(TaskSubcommand),

    /// Administrative governance, rollback, and reverification operations.
    #[command(subcommand)]
    Admin(AdminSubcommand),

    /// Multi-agent workspace container operations (create, list, inspect, merge).
    #[command(subcommand)]
    Workspace(WorkspaceSubcommand),

    /// Launch or inspect the Real-Time Substrate Web Explorer.
    #[command(subcommand)]
    Explorer(ExplorerSubcommand),
}

/// Returns the default greeting message.
#[must_use]
pub fn greeting() -> &'static str {
    "Hello World!"
}

/// Initializes tracing subscriber sending logs strictly to stderr.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
}

/// Main entry point logic for the CLI application.
///
/// # Errors
///
/// Returns an error if migration, server, or client command fails.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cli = Cli::parse();
    run_with_args(cli).await
}

/// Executes CLI logic with pre-parsed arguments.
///
/// # Errors
///
/// Returns an error if migration, server, or client command fails.
pub async fn run_with_args(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Top-level --migrate-only flag execution
    if cli.migrate_only {
        let database_url = cli.database_url.unwrap_or_else(db::resolve_database_url);

        tracing::info!("Connecting to database for migrations...");
        let (mut client, _handle) = db::connect(&database_url).await?;

        tracing::info!("Running database migrations...");
        let report = db::run_migrations(&mut client).await?;

        for migration in report.applied_migrations() {
            let version = migration.version();
            let name = migration.name();
            let file_name = format!("V{version}__{name}.sql");
            tracing::info!("Applied {file_name}");
            eprintln!("Applied {file_name}");
        }

        tracing::info!("Migrations completed successfully.");
        eprintln!("Migrations completed successfully.");
        return Ok(());
    }

    match cli.command {
        Some(Commands::Serve(mut args)) => {
            if args.database_url.is_none() && cli.database_url.is_some() {
                args.database_url = cli.database_url;
            }
            run_serve(args).await
        }

        Some(Commands::Ingest(args)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_ingest(args, &server_url, &auth_token).await
        }

        Some(Commands::McpStdio(args)) => run_mcp_stdio(args).await,

        Some(Commands::Staging(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_staging(subcmd, &server_url, &auth_token).await
        }

        Some(Commands::Identity(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_identity(subcmd, &server_url, &auth_token).await
        }

        Some(Commands::Task(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_task(subcmd, &server_url, &auth_token).await
        }

        Some(Commands::Admin(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_admin(subcmd, &server_url, &auth_token).await
        }

        Some(Commands::Workspace(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_workspace(subcmd, &server_url, &auth_token).await
        }

        Some(Commands::Explorer(subcmd)) => {
            let server_url = resolve_server_url(cli.server_url.as_deref());
            let auth_token = resolve_auth_token(cli.auth_token.as_deref());
            run_explorer(subcmd, &server_url, &auth_token).await
        }

        None => {
            println!("{}", greeting());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greeting() {
        assert_eq!(greeting(), "Hello World!");
    }

    #[tokio::test]
    async fn test_run_default() {
        let cli = Cli {
            migrate_only: false,
            database_url: None,
            server_url: None,
            auth_token: None,
            command: None,
        };
        assert!(run_with_args(cli).await.is_ok());
    }
}
