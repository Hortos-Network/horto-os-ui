//! `horto-os-ui-mcp` - Horto MCP server (stdio or Streamable HTTP).
//!
//! ```bash
//! horto-os-ui-mcp
//! MCP_HTTP=true HORTO_MCP_TOKEN=secret horto-os-ui-mcp --http
//! HORTO_MCP_MODE=box MCP_HTTP=true horto-os-ui-mcp --http --listen 0.0.0.0:8790
//! ```

use anyhow::Result;
use clap::Parser;
use horto_os_ui_mcp::{
    server::{default_listen_for, run_http, HortoMcp},
    McpSettings,
};
use horto_os_ui_shared::init_tracing;
use rmcp::{transport::stdio, ServiceExt};

#[derive(Debug, Parser)]
#[command(
    name = "horto-os-ui-mcp",
    about = "Horto OS UI MCP server (stdio or Streamable HTTP)",
    version
)]
struct Cli {
    /// Serve Streamable HTTP instead of stdio.
    #[arg(
        long,
        env = "MCP_HTTP",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    http: bool,

    /// HTTP bind address when `--http` is set (also: `HORTO_MCP_ADDR`).
    #[arg(long, env = "HORTO_MCP_ADDR")]
    listen: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Default `warn` keeps stdio MCP quiet for AI hosts; override with RUST_LOG.
    init_tracing("warn");
    let cli = Cli::parse();
    let settings = McpSettings::from_env();

    if cli.http {
        let listen = cli
            .listen
            .unwrap_or_else(|| default_listen_for(settings.mode).to_owned());
        tracing::info!(addr = %listen, mode = ?settings.mode, "horto-os-ui-mcp starting (HTTP)");
        run_http(&listen, settings)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
    } else {
        tracing::info!(mode = ?settings.mode, "horto-os-ui-mcp starting (stdio)");
        let service = HortoMcp::new(settings)
            .serve(stdio())
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        service
            .waiting()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
    }
    Ok(())
}
