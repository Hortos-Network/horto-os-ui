use anyhow::{bail, Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use clap::Parser;
use horto_os_ui_shared::{box_status, footer_line, HostContext, SetupKind, LONG_VERSION};
use serde::Serialize;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-status-api",
    about = "Horto OS UI box-local status API",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    /// Bind host:port. Prefer `localhost:8787` (resolves and listens on every
    /// loopback address, IPv4 and IPv6). IP literals still work when needed.
    #[arg(long, default_value = "localhost:8787", env = "HORTO_API_BIND")]
    bind: String,
    #[arg(long, env = "HORTO_API_TOKEN")]
    token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    token: Option<String>,
}

#[derive(Serialize)]
struct Health {
    ok: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "horto_os_ui_status_api=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    info!("{}", footer_line());
    if cli.token.is_none() {
        warn!("HORTO_API_TOKEN unset; API is open on the bind address (LAN early-use mode)");
    }

    let state = Arc::new(AppState { token: cli.token });
    // Browser / Tauri webview origins differ from http://localhost:8787, so a
    // CORS allowlist is required for `fetch`. This is not "open to the world":
    // only localhost + Tauri desktop origins. Real auth is HORTO_API_TOKEN.
    let cors = local_desktop_cors();

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addrs = resolve_bind(&cli.bind)?;
    let mut addrs = addrs.into_iter();
    let first = addrs
        .next()
        .ok_or_else(|| anyhow::anyhow!("no bind addresses for {}", cli.bind))?;
    let first_listener = tokio::net::TcpListener::bind(first)
        .await
        .with_context(|| format!("bind {first}"))?;
    info!("listening on {}", listen_url(first));

    for addr in addrs {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                info!("listening on {}", listen_url(addr));
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener, app).await {
                        warn!("listener exited: {e}");
                    }
                });
            }
            Err(e) => warn!("skip bind {addr}: {e}"),
        }
    }

    axum::serve(first_listener, app).await?;
    Ok(())
}

/// CORS limited to local desktop / loopback UI origins (not `*` / Any).
fn local_desktop_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _request| {
            is_local_desktop_origin(origin)
        }))
        .allow_methods([Method::GET, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
        .max_age(std::time::Duration::from_secs(600))
}

/// True for Tauri webview and localhost pages talking to this API.
fn is_local_desktop_origin(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    origin == "null"
        || origin.starts_with("http://localhost")
        || origin.starts_with("https://localhost")
        || origin.starts_with("http://tauri.localhost")
        || origin.starts_with("https://tauri.localhost")
        || origin.starts_with("tauri://localhost")
}

/// Resolve a clap bind string to one or more listen addresses.
fn resolve_bind(bind: &str) -> Result<Vec<SocketAddr>> {
    if let Ok(addr) = bind.parse::<SocketAddr>() {
        return Ok(vec![addr]);
    }
    let mut addrs: Vec<SocketAddr> = bind
        .to_socket_addrs()
        .with_context(|| format!("resolve bind {bind}"))?
        .collect();
    addrs.sort_unstable();
    addrs.dedup();
    if addrs.is_empty() {
        bail!("bind {bind} resolved to no addresses");
    }
    Ok(addrs)
}

fn listen_url(addr: SocketAddr) -> String {
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        format!("http://localhost:{}", addr.port())
    } else {
        format!("http://{addr}")
    }
}

fn bearer_authorized(expected: &str, header: Option<&str>) -> bool {
    header
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == expected)
}

async fn health() -> Json<Health> {
    Json(Health { ok: true })
}

async fn status() -> impl IntoResponse {
    let ctx = HostContext::new(horto_os_ui_shared::ApplyMode::DryRun, SetupKind::Full);
    let kind = if ctx.paths.minimal_env_file().exists() && !ctx.paths.full_env_file().exists() {
        SetupKind::Minimal
    } else {
        SetupKind::Full
    };
    let report = box_status(&ctx, kind);
    Json(report)
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // /health is always open
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let Some(ref expected) = state.token else {
        return next.run(req).await;
    };
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if bearer_authorized(expected, header) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_bind_and_token() {
        let cli = Cli::try_parse_from([
            "horto-os-ui-status-api",
            "--bind",
            "localhost:9999",
            "--token",
            "secret",
        ])
        .unwrap();
        assert_eq!(cli.bind, "localhost:9999");
        assert_eq!(cli.token.as_deref(), Some("secret"));
    }

    #[test]
    fn resolve_localhost_yields_loopback() {
        let addrs = resolve_bind("localhost:8787").unwrap();
        assert!(!addrs.is_empty());
        assert!(addrs.iter().all(|a| a.ip().is_loopback()));
        assert!(addrs.iter().all(|a| a.port() == 8787));
    }

    #[test]
    fn resolve_ipv4_literal() {
        let addrs = resolve_bind("127.0.0.1:8787").unwrap();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn listen_url_loopback_and_lan() {
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);
        assert_eq!(listen_url(loopback), "http://localhost:8787");
        let any = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8787);
        assert_eq!(listen_url(any), "http://localhost:8787");
        let lan = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 8787);
        assert_eq!(listen_url(lan), "http://192.168.1.10:8787");
    }

    #[test]
    fn bearer_auth_helper() {
        assert!(bearer_authorized("sec", Some("Bearer sec")));
        assert!(!bearer_authorized("sec", Some("Bearer other")));
        assert!(!bearer_authorized("sec", Some("Basic sec")));
        assert!(!bearer_authorized("sec", None));
    }

    #[test]
    fn local_origin_allowlist() {
        assert!(is_local_desktop_origin(&HeaderValue::from_static(
            "http://tauri.localhost"
        )));
        assert!(is_local_desktop_origin(&HeaderValue::from_static(
            "http://localhost:4187"
        )));
        assert!(!is_local_desktop_origin(&HeaderValue::from_static(
            "https://evil.example"
        )));
        let _ = local_desktop_cors();
    }
}
