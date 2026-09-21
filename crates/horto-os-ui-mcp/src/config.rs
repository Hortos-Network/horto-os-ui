//! Runtime settings from the environment.

use std::env;
use std::path::PathBuf;

/// Where the MCP process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpMode {
    /// PC: day-2 HTTP to status-api + privileged tools via OpenSSH remote runner.
    Pc,
    /// Box: day-2 HTTP to loopback status-api + privileged tools in-process.
    Box,
}

impl McpMode {
    /// Parse `HORTO_MCP_MODE` (`pc` / `box`). Default: `pc`.
    #[must_use]
    pub fn from_env() -> Self {
        match env::var("HORTO_MCP_MODE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "box" => Self::Box,
            _ => Self::Pc,
        }
    }
}

/// Settings shared by tools and HTTP server.
#[derive(Debug, Clone)]
pub struct McpSettings {
    /// PC vs box execution mode (`HORTO_MCP_MODE`).
    pub mode: McpMode,
    /// Base URL of `horto-os-ui-status-api` (`HORTO_STATUS_API_URL`).
    pub status_api_url: String,
    /// Status-api bearer (`HORTO_API_TOKEN`).
    pub api_token: Option<String>,
    /// Bearer required for Streamable HTTP. Falls back to `api_token`.
    pub mcp_token: Option<String>,
    /// SSH target host for PC privileged tools (`HORTO_REMOTE_HOST`).
    pub remote_host: Option<String>,
    /// Release tag for remote CLI install (`HORTO_RELEASE_TAG`).
    pub release_tag: Option<String>,
    /// Local directory of ecosystem binaries (`HORTO_BIN_DIR`).
    pub bin_dir: Option<PathBuf>,
    /// When true, offer SSH key install on remote probe (`HORTO_INSTALL_SSH_KEY`).
    pub install_ssh_key: bool,
}

impl McpSettings {
    /// Load from process environment.
    #[must_use]
    pub fn from_env() -> Self {
        let mode = McpMode::from_env();
        let status_api_url = status_api_url_from_env(mode);
        let api_token = env::var("HORTO_API_TOKEN")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let mcp_token = env::var("HORTO_MCP_TOKEN")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| api_token.clone());
        let remote_host = env::var("HORTO_REMOTE_HOST")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let release_tag = env::var("HORTO_RELEASE_TAG")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let bin_dir = env::var("HORTO_BIN_DIR")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let install_ssh_key = matches!(
            env::var("HORTO_INSTALL_SSH_KEY")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        );
        Self {
            mode,
            status_api_url,
            api_token,
            mcp_token,
            remote_host,
            release_tag,
            bin_dir,
            install_ssh_key,
        }
    }

    /// Token required when serving Streamable HTTP.
    #[must_use]
    pub fn http_bearer(&self) -> Option<&str> {
        self.mcp_token.as_deref()
    }
}

/// `HORTO_STATUS_API_URL`, else mode default.
fn status_api_url_from_env(mode: McpMode) -> String {
    env::var("HORTO_STATUS_API_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| match mode {
            McpMode::Box => "http://127.0.0.1:8787".into(),
            McpMode::Pc => "http://localhost:8787".into(),
        })
}
