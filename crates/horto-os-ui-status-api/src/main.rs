use anyhow::{bail, Context, Result};
use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use clap::Parser;
use horto_os_ui_shared::{box_status, footer_line, HostContext, SetupKind, LONG_VERSION};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
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
    /// Bind host:port. Default `0.0.0.0:8787` listens on all IPv4 interfaces so
    /// the box hostname / LAN IP can reach the API (not only loopback).
    /// Peer addresses are still restricted to loopback / private / link-local.
    #[arg(long, default_value = "0.0.0.0:8787", env = "HORTO_API_BIND")]
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
        warn!("HORTO_API_TOKEN unset; API accepts unauthenticated local-network clients");
    }
    info!("rejecting non-local client IPs (loopback / RFC1918 / ULA / link-local only)");

    let state = Arc::new(AppState { token: cli.token });
    // Browser / Tauri webview origins differ from the API origin, so a CORS
    // allowlist is required for `fetch`. Real auth is HORTO_API_TOKEN; peer IPs
    // must still be local-network (see local_net_middleware).
    let cors = local_desktop_cors();

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn(local_net_middleware))
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
                    let svc = app.into_make_service_with_connect_info::<SocketAddr>();
                    if let Err(e) = axum::serve(listener, svc).await {
                        warn!("listener exited: {e}");
                    }
                });
            }
            Err(e) => warn!("skip bind {addr}: {e}"),
        }
    }

    let svc = app.into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(first_listener, svc).await?;
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

/// True for loopback, RFC1918, IPv6 ULA, and link-local peers.
#[must_use]
fn is_local_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_ipv4(v4),
        IpAddr::V6(v6) => is_local_ipv6(v6),
    }
}

fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
    // rare; treat as local peer quirk
}

fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unicast_link_local() {
        return true;
    }
    // Unique local addresses fc00::/7 (includes fd00::/8).
    let octets = ip.octets();
    (octets[0] & 0xfe) == 0xfc
        // IPv4-mapped ::ffff:a.b.c.d → judge the embedded v4.
        || ip
            .to_ipv4_mapped()
            .is_some_and(is_local_ipv4)
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
    if addr.ip().is_unspecified() {
        format!("http://0.0.0.0:{} (all interfaces)", addr.port())
    } else if addr.ip().is_loopback() {
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

async fn local_net_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if is_local_network_ip(addr.ip()) {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "client address is not on a local network",
        )
            .into_response()
    }
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // /health is always open (still behind local_net_middleware).
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
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
        assert_eq!(listen_url(any), "http://0.0.0.0:8787 (all interfaces)");
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

    #[test]
    fn local_network_ip_allowlist() {
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 50
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 5, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));

        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        let ula: Ipv6Addr = "fd12:3456:789a::1".parse().unwrap();
        assert!(is_local_network_ip(IpAddr::V6(ula)));
        let link_local: Ipv6Addr = "fe80::1".parse().unwrap();
        assert!(is_local_network_ip(IpAddr::V6(link_local)));
        let global: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert!(!is_local_network_ip(IpAddr::V6(global)));

        let mapped_private =
            Ipv6Addr::from_octets([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 0, 1]);
        assert!(is_local_network_ip(IpAddr::V6(mapped_private)));
        let mapped_public =
            Ipv6Addr::from_octets([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 8, 8, 8, 8]);
        assert!(!is_local_network_ip(IpAddr::V6(mapped_public)));
    }
}
