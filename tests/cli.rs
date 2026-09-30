//! Integration test suite for CLI entry points (WP-1.6).

use std::process::Command;
use tks::cli::ServeArgs;
use tks::{Cli, Commands, greeting, run_with_args};

#[test]
fn test_greeting() {
    assert_eq!(greeting(), "Hello World!");
}

#[tokio::test]
async fn test_cli_default_invocation() {
    let cli = Cli {
        migrate_only: false,
        database_url: None,
        server_url: None,
        auth_token: None,
        command: None,
    };
    assert!(run_with_args(cli).await.is_ok());
}

#[tokio::test]
async fn test_cli_top_level_migrate_only() {
    let cli = Cli {
        migrate_only: true,
        database_url: None,
        server_url: None,
        auth_token: None,
        command: None,
    };
    assert!(run_with_args(cli).await.is_ok());
}

#[tokio::test]
async fn test_cli_serve_migrate_only() {
    let cli = Cli {
        migrate_only: false,
        database_url: None,
        server_url: None,
        auth_token: None,
        command: Some(Commands::Serve(ServeArgs {
            port: 8080,
            migrate_only: true,
            database_url: None,
            git_dir: None,
        })),
    };
    assert!(run_with_args(cli).await.is_ok());
}

#[test]
fn test_cli_binary_help_flags() {
    let outputs = [
        Command::new("cargo")
            .args(["run", "--quiet", "--bin", "tks", "--", "--help"])
            .output()
            .expect("Failed to execute tks --help"),
        Command::new("cargo")
            .args(["run", "--quiet", "--bin", "tks", "--", "serve", "--help"])
            .output()
            .expect("Failed to execute tks serve --help"),
        Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "--bin",
                "tks",
                "--",
                "mcp-stdio",
                "--help",
            ])
            .output()
            .expect("Failed to execute tks mcp-stdio --help"),
        Command::new("cargo")
            .args(["run", "--quiet", "--bin", "tks", "--", "staging", "--help"])
            .output()
            .expect("Failed to execute tks staging --help"),
        Command::new("cargo")
            .args(["run", "--quiet", "--bin", "tks", "--", "identity", "--help"])
            .output()
            .expect("Failed to execute tks identity --help"),
        Command::new("cargo")
            .args(["run", "--quiet", "--bin", "mcp_stdio", "--", "--help"])
            .output()
            .expect("Failed to execute mcp_stdio --help"),
    ];

    for out in &outputs {
        assert!(out.status.success(), "CLI help flag must exit 0");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Usage:") || stdout.contains("Commands:"),
            "Help output must display usage information"
        );
    }
}
