use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
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

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct BoxStatus {
    pub hostname: String,
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

fn auth_header(token: Option<&str>) -> Option<String> {
    token
        .map(str::trim)
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
             Enter the same bearer token as HORTO_API_TOKEN on the box, or clear the token if the API has none."
        ),
        403 => format!("{endpoint} at {url} returned HTTP 403 Forbidden."),
        404 => format!(
            "{endpoint} at {url} returned HTTP 404. \
             Confirm the Status API base URL (no extra path) and that this build exposes {endpoint}."
        ),
        code if (500..600).contains(&code) => {
            format!("{endpoint} at {url} returned HTTP {code} (server error on the box).")
        }
        code => format!("{endpoint} at {url} returned HTTP {code}."),
    }
}

async fn fetch_health(base: &str) -> Result<bool, String> {
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
    Ok(true)
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
        Ok(ok) => snap.health_ok = Some(ok),
        Err(e) => {
            snap.error = Some(e);
            return snap;
        }
    }
    match fetch_status(&base, token.as_deref()).await {
        Ok(st) => snap.status = Some(st),
        Err(e) => snap.error = Some(e),
    }
    snap
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
