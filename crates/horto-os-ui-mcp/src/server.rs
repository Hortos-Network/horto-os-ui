//! MCP server (`rmcp`) for Horto (stdio or Streamable HTTP).

#![allow(clippy::unused_async)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use subtle::ConstantTimeEq;
use tokio::task::spawn_blocking;

use crate::api_client::StatusApiClient;
use crate::config::{McpMode, McpSettings};
use crate::embedded_ops;
use crate::net::is_local_network_ip;
use crate::remote_ops;
use crate::tool_args::{BackupEtcArgs, DockerRebuildArgs, SetupRunArgs, SetupStepArgs};

/// Default HTTP listen when `HORTO_MCP_MODE=pc`.
pub const DEFAULT_HTTP_LISTEN_PC: &str = "127.0.0.1:8790";
/// Default HTTP listen when `HORTO_MCP_MODE=box`.
pub const DEFAULT_HTTP_LISTEN_BOX: &str = "0.0.0.0:8790";

/// MCP server handle.
#[derive(Clone)]
pub struct HortoMcp {
    settings: Arc<McpSettings>,
}

impl HortoMcp {
    /// Build from settings.
    #[must_use]
    pub fn new(settings: McpSettings) -> Self {
        Self {
            settings: Arc::new(settings),
        }
    }

    fn settings(&self) -> Arc<McpSettings> {
        Arc::clone(&self.settings)
    }
}

pub(crate) fn text_ok(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

pub(crate) fn mcp_err(msg: impl Into<String>) -> McpError {
    McpError::invalid_params(msg.into(), None)
}

fn to_json_text<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    serde_json::to_string_pretty(value).map_err(|err| mcp_err(err.to_string()))
}

async fn blocking_str<F>(f: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> anyhow::Result<String> + Send + 'static,
{
    spawn_blocking(f)
        .await
        .map_err(|err| mcp_err(format!("join error: {err}")))?
        .map(text_ok)
        .map_err(|err| mcp_err(err.to_string()))
}

#[tool_router]
impl HortoMcp {
    /// `GET /health` on the status API.
    #[tool(description = "Horto status-api health (GET /health)")]
    async fn health(&self) -> Result<CallToolResult, McpError> {
        let client =
            StatusApiClient::from_settings(&self.settings).map_err(|e| mcp_err(e.to_string()))?;
        let v = client.health().await.map_err(|e| mcp_err(e.to_string()))?;
        Ok(text_ok(to_json_text(&v)?))
    }

    /// `GET /v1/status` (bearer when token set).
    #[tool(description = "Horto box status snapshot (GET /v1/status)")]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let client =
            StatusApiClient::from_settings(&self.settings).map_err(|e| mcp_err(e.to_string()))?;
        let v = client.status().await.map_err(|e| mcp_err(e.to_string()))?;
        Ok(text_ok(to_json_text(&v)?))
    }

    /// Confirmed timestamped `/etc` backup via status-api.
    #[tool(
        description = "Timestamped /etc backup via status-api (confirm must be backup-etc; needs HORTO_API_TOKEN)"
    )]
    async fn backup_etc(
        &self,
        Parameters(args): Parameters<BackupEtcArgs>,
    ) -> Result<CallToolResult, McpError> {
        let client =
            StatusApiClient::from_settings(&self.settings).map_err(|e| mcp_err(e.to_string()))?;
        let v = client
            .backup_etc(&args.confirm)
            .await
            .map_err(|e| mcp_err(e.to_string()))?;
        Ok(text_ok(to_json_text(&v)?))
    }

    /// Probe remote box arch (PC mode only).
    #[tool(description = "Probe box arch over SSH (HORTO_MCP_MODE=pc; needs HORTO_REMOTE_HOST)")]
    async fn remote_probe(&self) -> Result<CallToolResult, McpError> {
        if self.settings.mode != McpMode::Pc {
            return Err(mcp_err(
                "remote_probe is only available in HORTO_MCP_MODE=pc",
            ));
        }
        let settings = self.settings();
        blocking_str(move || remote_ops::remote_probe(&settings)).await
    }

    /// Setup status (remote SSH or embedded).
    #[tool(description = "Setup pipeline status (remote SSH or embedded on box)")]
    async fn setup_status(
        &self,
        Parameters(args): Parameters<SetupRunArgs>,
    ) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => {
                blocking_str(move || remote_ops::setup_status(&settings, args.apply, args.full))
                    .await
            }
            McpMode::Box => {
                blocking_str(move || embedded_ops::setup_status_report(args.apply, args.full)).await
            }
        }
    }

    /// Full or minimal setup run.
    #[tool(
        description = "Run setup pipeline (apply default false = plan only). PC=SSH remote runner; box=embedded engine"
    )]
    async fn setup_run(
        &self,
        Parameters(args): Parameters<SetupRunArgs>,
    ) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => {
                blocking_str(move || {
                    remote_ops::setup_run(&settings, args.apply, args.full, args.skip_piper)
                })
                .await
            }
            McpMode::Box => {
                blocking_str(move || {
                    embedded_ops::setup_run_embedded(args.apply, args.full, args.skip_piper)
                })
                .await
            }
        }
    }

    /// Single setup step.
    #[tool(description = "Run one setup step by id (s1, m1, d1, …)")]
    async fn setup_step(
        &self,
        Parameters(args): Parameters<SetupStepArgs>,
    ) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        let step_id = args.step_id.clone();
        match settings.mode {
            McpMode::Pc => {
                blocking_str(move || {
                    remote_ops::setup_step(&settings, &step_id, args.apply, args.full)
                })
                .await
            }
            McpMode::Box => {
                blocking_str(move || {
                    embedded_ops::setup_step_embedded(&step_id, args.apply, args.full)
                })
                .await
            }
        }
    }

    /// Doctor report.
    #[tool(description = "Doctor / readiness report (remote or embedded)")]
    async fn doctor(&self) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => blocking_str(move || remote_ops::doctor(&settings)).await,
            McpMode::Box => blocking_str(embedded_ops::doctor_embedded).await,
        }
    }

    /// Docker container list / status.
    #[tool(description = "Docker status (remote or embedded)")]
    async fn docker_status(&self) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => blocking_str(move || remote_ops::docker_status(&settings)).await,
            McpMode::Box => blocking_str(embedded_ops::docker_status_embedded).await,
        }
    }

    /// Docker compose rebuild (confirm `docker-rebuild`).
    #[tool(description = "Docker compose rebuild (confirm must be docker-rebuild)")]
    async fn docker_rebuild(
        &self,
        Parameters(args): Parameters<DockerRebuildArgs>,
    ) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        let confirm = args.confirm.clone();
        match settings.mode {
            McpMode::Pc => {
                blocking_str(move || remote_ops::docker_rebuild(&settings, &confirm)).await
            }
            McpMode::Box => {
                blocking_str(move || embedded_ops::docker_rebuild_embedded(&confirm)).await
            }
        }
    }

    /// List timestamped `/etc` backups.
    #[tool(description = "List timestamped /etc backups (remote or embedded)")]
    async fn backup_list(&self) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => blocking_str(move || remote_ops::backup_list(&settings)).await,
            McpMode::Box => blocking_str(embedded_ops::backup_list_embedded).await,
        }
    }

    /// Disk backup probe / status.
    #[tool(description = "Disk backup readiness / status (remote or embedded)")]
    async fn backup_disk_status(&self) -> Result<CallToolResult, McpError> {
        let settings = self.settings();
        match settings.mode {
            McpMode::Pc => blocking_str(move || remote_ops::backup_disk_status(&settings)).await,
            McpMode::Box => blocking_str(embedded_ops::backup_disk_status_embedded).await,
        }
    }
}

#[tool_handler]
impl ServerHandler for HortoMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "horto-os-ui",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Horto tools: health, get_status, backup_etc (HTTP day-2); setup_*, doctor, docker_*, backup_list, backup_disk_status (SSH on pc / embedded on box). Set HORTO_MCP_MODE, HORTO_STATUS_API_URL, HORTO_API_TOKEN; PC needs HORTO_REMOTE_HOST. HTTP mode requires HORTO_MCP_TOKEN (or API token) and LAN peers only.",
            )
    }
}

#[derive(Clone)]
struct HttpGate {
    token: String,
}

fn bearer_ok(expected: &str, header: Option<&str>) -> bool {
    let Some(provided) = header.and_then(|v| v.strip_prefix("Bearer ")) else {
        return false;
    };
    if provided.len() != expected.len() {
        return false;
    }
    bool::from(provided.as_bytes().ct_eq(expected.as_bytes()))
}

async fn gate_middleware(
    State(gate): State<HttpGate>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !is_local_network_ip(addr.ip()) {
        return (StatusCode::FORBIDDEN, "forbidden: non-LAN peer").into_response();
    }
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if !bearer_ok(&gate.token, auth) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    next.run(req).await
}

fn http_router(settings: McpSettings) -> Result<Router, String> {
    let token = settings
        .http_bearer()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "HTTP mode requires HORTO_MCP_TOKEN or HORTO_API_TOKEN".to_owned())?
        .to_owned();
    let mcp = HortoMcp::new(settings);
    let config =
        rmcp::transport::streamable_http_server::tower::StreamableHttpServerConfig::default();
    let service = rmcp::transport::streamable_http_server::tower::StreamableHttpService::new(
        move || Ok(mcp.clone()),
        Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        config,
    );
    let method_router = axum::routing::any_service(service);
    let gate = HttpGate { token };
    Ok(Router::new()
        .route("/mcp", method_router.clone())
        .route("/mcp/", method_router)
        .layer(from_fn_with_state(gate, gate_middleware)))
}

/// Serve Streamable HTTP until stopped.
///
/// # Errors
///
/// Returns bind/serve I/O errors, or missing token configuration.
pub async fn run_http(addr: &str, settings: McpSettings) -> Result<(), String> {
    let router = http_router(settings)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    tracing::info!(%local, "horto-os-ui-mcp HTTP listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Default listen string for a mode.
#[must_use]
pub fn default_listen_for(mode: McpMode) -> &'static str {
    match mode {
        McpMode::Pc => DEFAULT_HTTP_LISTEN_PC,
        McpMode::Box => DEFAULT_HTTP_LISTEN_BOX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_constant_time_compare() {
        assert!(bearer_ok("secret", Some("Bearer secret")));
        assert!(!bearer_ok("secret", Some("Bearer wrong")));
        assert!(!bearer_ok("secret", Some("secret")));
        assert!(!bearer_ok("secret", None));
    }

    #[tokio::test]
    async fn http_requires_token_config() {
        let settings = McpSettings {
            mode: McpMode::Pc,
            status_api_url: "http://127.0.0.1:8787".into(),
            api_token: None,
            mcp_token: None,
            remote_host: None,
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        let err = http_router(settings).expect_err("token");
        assert!(err.contains("HORTO_MCP_TOKEN") || err.contains("HORTO_API_TOKEN"));
    }

    #[tokio::test]
    async fn http_rejects_missing_bearer() {
        let settings = McpSettings {
            mode: McpMode::Pc,
            status_api_url: "http://127.0.0.1:8787".into(),
            api_token: Some("tok".into()),
            mcp_token: Some("tok".into()),
            remote_host: None,
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        let router = http_router(settings).expect("router");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
        });

        let res = reqwest::Client::new()
            .post(format!("http://{addr}/mcp"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body("{}")
            .send()
            .await
            .expect("post");
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        server.abort();
        let _ = server.await;
    }
}
