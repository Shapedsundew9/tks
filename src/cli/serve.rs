//! Server daemon CLI subcommand implementation (WP-1.6, C-12, D-56, D-19, D-53).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Args;
use tokio_util::sync::CancellationToken;

use crate::db;
use crate::gateway::{AppState, run_server};
use crate::storage::StorageRepo;
use crate::storage::git::{resolve_git_dir, start_git_actor};
use crate::worker::start_worker_manager;

/// Arguments for `tks serve`.
#[derive(Debug, Args, Clone)]
pub struct ServeArgs {
    /// Port to bind the unified HTTP/MCP server (default: 8080).
    #[arg(long, default_value = "8080")]
    pub port: u16,

    /// Run pending database migrations against $DATABASE_URL and exit immediately.
    #[arg(long)]
    pub migrate_only: bool,

    /// Optional database connection URL (overrides $DATABASE_URL).
    #[arg(long)]
    pub database_url: Option<String>,

    /// Git repository path (overrides TKS_GIT_DIR).
    #[arg(long)]
    pub git_dir: Option<PathBuf>,
}

/// Executes the `tks serve` daemon.
///
/// # Errors
///
/// Returns an error if migrations, database connection, git actor, or socket binding fails.
pub async fn run_serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let database_url = args.database_url.unwrap_or_else(db::resolve_database_url);

    // Step 1: Run pending embedded refinery migrations on boot (C-12, D-56)
    tracing::info!("Connecting to PostgreSQL database...");
    let (mut client, _handle) = db::connect(&database_url).await?;

    tracing::info!("Running embedded database migrations...");
    let report = db::run_migrations(&mut client).await?;

    for migration in report.applied_migrations() {
        let version = migration.version();
        let name = migration.name();
        let file_name = format!("V{version}__{name}.sql");
        tracing::info!("Applied {file_name}");
        eprintln!("Applied {file_name}");
    }

    // Ensure default development identity exists idempotently (V2 seed)
    let _ = client
        .execute(
            "INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active) \
             VALUES ('tks_dev_token', 'tks_dev_token', 'HUMAN', TRUE) \
             ON CONFLICT (agent_id) DO UPDATE SET is_active = TRUE;",
            &[],
        )
        .await;

    if args.migrate_only {
        tracing::info!("Standalone database migrations completed successfully.");
        eprintln!("Standalone database migrations completed successfully.");
        return Ok(());
    }

    // Step 2: Initialize bare Git repository and start write actor supervisor (WP-1.1, TB-1)
    let git_dir = args.git_dir.unwrap_or_else(resolve_git_dir);
    tracing::info!(
        "Initializing bare Git repository at '{}'...",
        git_dir.display()
    );
    let (git_write, git_read) = start_git_actor(&git_dir, 256)?;

    // Step 3: Create connection pool and storage repository (WP-1.2)
    let pool = db::create_pool(&database_url).map_err(|e| e.to_string())?;
    let storage = StorageRepo::new(pool.clone());

    // Step 4: Initialize shared gateway application state and event bus (WP-1.5, WP-3.1)
    let cancel_token = CancellationToken::new();
    let event_bus = crate::storage::GraphEventBus::default();
    let _pg_listener_handle =
        crate::storage::start_pg_listener(&database_url, event_bus.clone(), cancel_token.clone());
    let state =
        AppState::new(pool.clone(), storage, git_write.clone(), git_read).with_event_bus(event_bus);

    // Step 5: Start background worker manager (WP-1.4, WP-3.1)
    let worker_token = cancel_token.clone();
    tracing::info!("Starting background decomposition, embedding, and cascade worker loops...");
    let _worker_handle = start_worker_manager(pool, git_write, worker_token).await;

    // Step 6: Spawn Ctrl-C listener for graceful shutdown coordination
    let shutdown_token = cancel_token.clone();
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            tracing::info!("Received Ctrl-C signal, coordinating graceful shutdown...");
            eprintln!("\nReceived Ctrl-C signal, shutting down TKS daemon...");
            shutdown_token.cancel();
        }
    });

    // Step 7: Bind TCP port and run Axum server (WP-1.5, D-19, D-53)
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    tracing::info!("Starting TKS daemon server on http://{addr}...");
    eprintln!("Starting TKS daemon server on http://{addr}...");
    run_server(addr, state, cancel_token).await?;

    Ok(())
}
