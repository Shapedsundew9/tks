//! TKS (The Knowledge Substrate) Library.

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod db;
pub mod ingest;
pub mod storage;

/// Command-line arguments for the TKS CLI.
#[derive(Parser, Debug)]
#[command(name = "tks", about = "The Knowledge Substrate")]
pub struct Cli {
    /// Run pending database migrations against $DATABASE_URL and exit.
    #[arg(long)]
    pub migrate_only: bool,

    /// Optional database connection URL (overrides $DATABASE_URL).
    #[arg(long)]
    pub database_url: Option<String>,
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
/// Returns an error if migration or database connection fails.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cli = Cli::parse();
    run_with_args(cli).await
}

/// Executes CLI logic with pre-parsed arguments.
///
/// # Errors
///
/// Returns an error if migration or database connection fails.
pub async fn run_with_args(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
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
        return Ok(());
    }

    println!("{}", greeting());
    Ok(())
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
        };
        assert!(run_with_args(cli).await.is_ok());
    }
}
