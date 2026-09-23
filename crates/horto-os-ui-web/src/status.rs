use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
    /// Box status-api tip long-version when present (`0.1.0 (abc1234)`).
    #[serde(default)]
    pub cli_version: String,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct ContainerInfo {
    pub names: String,
    #[serde(default)]
    pub image: String,
    pub status: String,
    /// Compose project / Dockge stack name when known.
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct UrlInfo {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub up: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct BackupStatus {
    pub initial_setup_present: bool,
    #[serde(default)]
    pub timestamped: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, serde::Serialize)]
pub struct HostMetrics {
    #[serde(default)]
    pub cpu_percent: Option<f32>,
    #[serde(default)]
    pub load_1: Option<f32>,
    #[serde(default)]
    pub load_5: Option<f32>,
    #[serde(default)]
    pub load_15: Option<f32>,
    #[serde(default)]
    pub disk_total_bytes: Option<u64>,
    #[serde(default)]
    pub disk_used_bytes: Option<u64>,
    #[serde(default)]
    pub disk_avail_bytes: Option<u64>,
    #[serde(default)]
    pub os_pretty_name: Option<String>,
    #[serde(default)]
    pub os_id: Option<String>,
    #[serde(default)]
    pub os_version_id: Option<String>,
    #[serde(default)]
    pub armbian_version: Option<String>,
    #[serde(default)]
    pub armbian_board: Option<String>,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub apt_upgradable: Option<u32>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct BoxStatus {
    #[serde(default)]
    pub cli_version: String,
    pub hostname: String,
    #[serde(default)]
    pub host: HostMetrics,
    #[serde(default)]
    pub containers: Vec<ContainerInfo>,
    #[serde(default)]
    pub urls: Vec<UrlInfo>,
    #[serde(default)]
    pub backup: BackupStatus,
}

#[derive(Clone, PartialEq)]
pub struct Snapshot {
    pub health_ok: Option<bool>,
    pub status: Option<BoxStatus>,
    /// Tip long-version from `/health` when the API answered.
    pub api_cli_version: Option<String>,
    pub error: Option<String>,
}

/// Short status line for the connection panel.
#[must_use]
pub fn connection_label(snap: &Snapshot) -> (String, &'static str) {
    if snap.error.is_some() {
        return ("Connection failed".into(), "status-bad");
    }
    match snap.health_ok {
        Some(true) => ("API healthy".into(), "status-ok"),
        Some(false) => ("API reported not ok".into(), "status-bad"),
        None => ("Not connected yet".into(), "status-warn"),
    }
}

/// Full error text for the alert (when present).
#[must_use]
pub fn connection_error_detail(snap: &Snapshot) -> Option<String> {
    snap.error.clone()
}

/// Rewrite a service link so its host matches the Status API URL (same box, same reachability).
#[must_use]
pub fn rewrite_service_url_host(service_url: &str, api_base: &str) -> String {
    let Ok(api) = web_sys::Url::new(api_base.trim()) else {
        return service_url.to_owned();
    };
    let Ok(svc) = web_sys::Url::new(service_url.trim()) else {
        return service_url.to_owned();
    };
    let host = api.hostname();
    if host.is_empty() {
        return service_url.to_owned();
    }
    svc.set_hostname(&host);
    svc.href()
}

/// Strip a pasted `Bearer ` prefix and whitespace so the header is exactly once.
#[must_use]
pub fn normalize_bearer_token(raw: &str) -> String {
    let trimmed = raw.trim();
    let without = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim();
    without.to_owned()
}

fn auth_header(token: Option<&str>) -> Option<String> {
    token
        .map(normalize_bearer_token)
        .filter(|t| !t.is_empty())
        .map(|t| format!("Bearer {t}"))
}

fn is_unreachable_browser_error(raw: &str) -> bool {
    let lower = raw.to_lowercase();
    lower.contains("failed to fetch")
        || lower.contains("networkerror")
        || lower.contains("load failed")
        || lower.contains("network request failed")
}

fn explain_unreachable(endpoint: &str, url: &str, raw: &str) -> String {
    format!(
        "Cannot reach {endpoint} at {url}. \
         Check that horto-os-ui-status-api is running and the Status API URL is correct. \
         If you used the box hostname, the API must accept connections on that address \
         (with a loopback-only bind, use http://localhost:8787). \
         Browser: {raw}"
    )
}

fn explain_http(endpoint: &str, url: &str, status: u16) -> String {
    match status {
        401 => format!(
            "{endpoint} at {url} returned HTTP 401 Unauthorized. \
             Horto must send the Status API token from the local token file (same as TUI / CLI)."
        ),
        403 => format!("{endpoint} at {url} returned HTTP 403 Forbidden."),
        404 => format!(
            "{endpoint} at {url} returned HTTP 404. \
             Confirm the Status API base URL (no extra path) and that this build exposes {endpoint}."
        ),
        503 => format!(
            "{endpoint} at {url} returned HTTP 503. \
             Mutate routes need HORTO_API_TOKEN configured on the box (remote install writes \
             /etc/horto-os-ui/api.env)."
        ),
        code if (500..600).contains(&code) => {
            format!("{endpoint} at {url} returned HTTP {code} (server error on the box).")
        }
        code => format!("{endpoint} at {url} returned HTTP {code}."),
    }
}

async fn fetch_health(base: &str) -> Result<(bool, Option<String>), String> {
    let health_url = format!("{base}/health");
    let resp = gloo_net::http::Request::get(&health_url)
        .send()
        .await
        .map_err(|e| {
            let raw = e.to_string();
            if is_unreachable_browser_error(&raw) {
                explain_unreachable("/health", &health_url, &raw)
            } else {
                format!("/health request to {health_url} failed: {raw}")
            }
        })?;
    if !resp.ok() {
        return Err(explain_http("/health", &health_url, resp.status()));
    }
    let health: Health = resp.json().await.map_err(|e| {
        format!("/health at {health_url} returned a body that is not valid JSON ({e}).")
    })?;
    if !health.ok {
        return Err(format!(
            "/health at {health_url} responded ok=false (API process is up but reports unhealthy)."
        ));
    }
    let cli_version = {
        let v = health.cli_version.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_owned())
        }
    };
    Ok((true, cli_version))
}

async fn fetch_status(base: &str, token: Option<&str>) -> Result<BoxStatus, String> {
    let status_url = format!("{base}/v1/status");
    let mut builder = gloo_net::http::Request::get(&status_url);
    if let Some(header) = auth_header(token) {
        builder = builder.header("Authorization", &header);
    }
    let resp = builder.send().await.map_err(|e| {
        let raw = e.to_string();
        if is_unreachable_browser_error(&raw) {
            explain_unreachable("/v1/status", &status_url, &raw)
        } else {
            format!("/v1/status request to {status_url} failed: {raw}")
        }
    })?;
    if !resp.ok() {
        return Err(explain_http("/v1/status", &status_url, resp.status()));
    }
    resp.json().await.map_err(|e| {
        format!("/v1/status at {status_url} returned a body that is not valid JSON ({e}).")
    })
}

pub async fn fetch_snapshot(base_url: String, token: Option<String>) -> Snapshot {
    let base = base_url.trim().trim_end_matches('/').to_string();
    let mut snap = Snapshot {
        health_ok: None,
        status: None,
        api_cli_version: None,
        error: None,
    };
    if base.is_empty() {
        snap.error = Some(
            "Status API URL is empty. Enter http://localhost:8787 \
             (or http://<box-hostname>:8787 when the API listens on the LAN)."
                .into(),
        );
        return snap;
    }
    if web_sys::Url::new(&base).is_err() {
        snap.error = Some(format!(
            "Status API URL is not a valid absolute URL: {base}. \
             Example: http://localhost:8787"
        ));
        return snap;
    }
    match fetch_health(&base).await {
        Ok((ok, cli_version)) => {
            snap.health_ok = Some(ok);
            snap.api_cli_version = cli_version;
        }
        Err(e) => {
            snap.error = Some(e);
            return snap;
        }
    }
    match fetch_status(&base, token.as_deref()).await {
        Ok(st) => {
            if snap.api_cli_version.is_none() {
                let v = st.cli_version.trim();
                if !v.is_empty() {
                    snap.api_cli_version = Some(v.to_owned());
                }
            }
            snap.status = Some(st);
        }
        Err(e) => snap.error = Some(e),
    }
    snap
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EtcBackupReport {
    pub dest: String,
    #[serde(default)]
    pub copied: Vec<String>,
}

/// POST timestamped `/etc` backup. Requires bearer token and confirm header.
pub async fn post_backup_etc(
    base_url: &str,
    token: Option<&str>,
) -> Result<EtcBackupReport, String> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/v1/backup/etc");
    let Some(auth) = auth_header(token) else {
        return Err(
            "Bearer token required for backup. Paste HORTO_API_TOKEN from the box into Connection."
                .into(),
        );
    };
    let resp = gloo_net::http::Request::post(&url)
        .header("Authorization", &auth)
        .header("X-Horto-Confirm", "backup-etc")
        .send()
        .await
        .map_err(|e| {
            let raw = e.to_string();
            if is_unreachable_browser_error(&raw) {
                explain_unreachable("/v1/backup/etc", &url, &raw)
            } else {
                format!("/v1/backup/etc request to {url} failed: {raw}")
            }
        })?;
    if !resp.ok() {
        return Err(explain_http("/v1/backup/etc", &url, resp.status()));
    }
    resp.json().await.map_err(|e| {
        format!("/v1/backup/etc at {url} returned a body that is not valid JSON ({e}).")
    })
}

/// Dockge deep link for a container stack (`/compose/<stack>`), same host as the API.
///
/// The Dockge container itself opens the Dockge home page.
#[must_use]
pub fn dockge_href_for_container(
    stack: Option<&str>,
    names: &str,
    urls: &[UrlInfo],
    api_base: &str,
) -> Option<String> {
    let dockge = urls.iter().find(|u| normalize_key(&u.name) == "dockge")?;
    let base = rewrite_service_url_host(&dockge.url, api_base)
        .trim_end_matches('/')
        .to_owned();
    let stack_name = stack
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| primary_container_name(names))?;
    if normalize_key(&stack_name) == "dockge" {
        return Some(base);
    }
    Some(format!(
        "{base}/compose/{}",
        encode_path_segment(&stack_name)
    ))
}

fn primary_container_name(names: &str) -> Option<String> {
    names
        .split(',')
        .map(str::trim)
        .map(|n| n.trim_start_matches('/'))
        .find(|n| !n.is_empty())
        .map(str::to_owned)
}

fn encode_path_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn normalize_key(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Desktop Services catalog (name, default port). Keep aligned with shared `SERVICE_CATALOG`.
/// Display order is alphabetical by name.
const SERVICE_CATALOG: &[(&str, u16)] = &[
    ("Cockpit", 9890),
    ("DeepSeek", 8001),
    ("Dockge", 5001),
    ("EVCC", 7070),
    ("Homepage", 3021),
    ("MCP", 8790),
    ("Open-WebUI", 3000),
    ("OpenWakeWord", 10400),
    ("Piper", 10200),
    ("Status-API", 8787),
    ("Whisper", 8000),
];

fn catalog_blurb(name: &str) -> Option<&'static str> {
    match normalize_key(name).as_str() {
        "homepage" => Some("Box dashboard (gethomepage) for apps and widgets."),
        "dockge" => Some("Compose stack manager for Docker apps on the box."),
        "cockpit" => Some("Host admin console (packages, logs, storage, network)."),
        "open-webui" => Some("Local chat UI for on-box LLM backends."),
        "evcc" => Some("Home energy manager (chargers, PV, battery)."),
        "whisper" => Some("Speech-to-text service used by voice pipelines."),
        "deepseek" => Some("Local LLM endpoint (often via Open-WebUI)."),
        "piper" => Some("Text-to-speech engine (Wyoming / voice stack)."),
        "openwakeword" => Some("Wake-word detection for hands-free voice."),
        "status-api" => Some("Horto Status API (box health, containers, service links)."),
        "mcp" => Some("Horto MCP server (HTTP tools for the box)."),
        _ => None,
    }
}

/// Full Services list: catalog defaults; API row wins when the name matches (live port / up).
///
/// `link_host` is the Connection page host (preferred). Falls back to the Status API URL host.
#[must_use]
pub fn merge_urls_with_catalog(
    api_urls: Vec<UrlInfo>,
    api_base: &str,
    link_host: &str,
) -> Vec<UrlInfo> {
    let (scheme, api_host) = scheme_host_from_api_base(api_base);
    let host = preferred_link_host(link_host, &api_host);
    let by_key: std::collections::BTreeMap<String, UrlInfo> = api_urls
        .into_iter()
        .map(|u| (normalize_key(&u.name), u))
        .collect();
    SERVICE_CATALOG
        .iter()
        .map(|(name, default_port)| {
            let key = normalize_key(name);
            if let Some(existing) = by_key.get(&key) {
                let mut u = existing.clone();
                u.url = rewrite_url_to_host(&u.url, &scheme, &host);
                if u.description.as_ref().is_none_or(|s| s.is_empty()) {
                    u.description = catalog_blurb(name).map(str::to_owned);
                }
                return u;
            }
            UrlInfo {
                name: (*name).to_owned(),
                url: format!("{scheme}://{host}:{default_port}"),
                up: false,
                description: catalog_blurb(name).map(str::to_owned),
            }
        })
        .collect()
}

/// Connection host when set; otherwise the host from the Status API URL.
#[must_use]
pub fn preferred_link_host(connection_host: &str, api_url_host: &str) -> String {
    let from_conn = hostname_from_connection(connection_host);
    if from_conn != "unknown" {
        from_conn
    } else if !api_url_host.trim().is_empty() {
        api_url_host.trim().to_owned()
    } else {
        "localhost".into()
    }
}

/// `user@host` → host; empty → `unknown`.
#[must_use]
pub fn hostname_from_connection(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "unknown".into();
    }
    raw.rsplit_once('@')
        .map(|(_, host)| host.trim())
        .filter(|h| !h.is_empty())
        .unwrap_or(raw)
        .to_owned()
}

/// True when Connection host is this PC (`localhost` / loopback).
#[must_use]
pub fn connection_host_is_local(raw: &str) -> bool {
    let host = hostname_from_connection(raw);
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "0.0.0.0"
}

/// Attach local Desktop host sensors when Connection is this PC.
///
/// Status API host metrics are for a remote box; on localhost Overview uses
/// `collect_host_metrics` via the Desktop bridge instead.
pub fn apply_local_host_metrics(snap: &mut Snapshot, metrics: HostMetrics, connection_host: &str) {
    let hostname = hostname_from_connection(connection_host);
    match &mut snap.status {
        Some(st) => {
            st.host = metrics;
            if st.hostname.trim().is_empty() || st.hostname.eq_ignore_ascii_case("unknown") {
                st.hostname = hostname;
            }
        }
        None => {
            snap.status = Some(BoxStatus {
                hostname,
                host: metrics,
                ..Default::default()
            });
        }
    }
}

fn rewrite_url_to_host(service_url: &str, scheme: &str, host: &str) -> String {
    let Ok(parsed) = web_sys::Url::new(service_url.trim()) else {
        return service_url.to_owned();
    };
    let port = parsed.port();
    if port.is_empty() {
        format!("{scheme}://{host}{}", parsed.pathname())
    } else {
        format!("{scheme}://{host}:{port}{}", parsed.pathname())
    }
}

fn scheme_host_from_api_base(api_base: &str) -> (String, String) {
    let s = api_base.trim();
    let (scheme, rest) = if let Some(r) = s.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = s.strip_prefix("http://") {
        ("http", r)
    } else {
        return ("http".into(), "localhost".into());
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let host = host_port
        .rsplit_once('@')
        .map_or(host_port, |(_, h)| h)
        .rsplit_once(':')
        .map_or(host_port, |(h, _)| h);
    if host.is_empty() {
        ("http".into(), "localhost".into())
    } else {
        (scheme.into(), host.into())
    }
}
