//! Standalone stdio streaming JSON-RPC proxy binary for MCP clients (WP-1.6, TB-4).

use clap::Parser;
use tks::cli::mcp_stdio::{McpStdioArgs, run_mcp_stdio};

#[derive(Parser, Debug)]
#[command(
    name = "mcp_stdio",
    about = "TKS Model Context Protocol Stdio Streaming Proxy"
)]
struct StdioCli {
    #[command(flatten)]
    args: McpStdioArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = StdioCli::parse();
    run_mcp_stdio(cli.args).await
}
