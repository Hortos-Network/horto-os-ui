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
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct UrlInfo {
    pub name: String,
    pub url: String,
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

pub fn connection_label(snap: &Snapshot) -> (String, &'static str) {
    if let Some(err) = &snap.error {
        return (format!("Error: {err}"), "status-bad");
    }
    match snap.health_ok {
        Some(true) => ("API healthy".into(), "status-ok"),
        Some(false) => ("API reported not ok".into(), "status-bad"),
        None => ("Not connected yet".into(), "status-warn"),
    }
}

fn auth_header(token: Option<&str>) -> Option<String> {
    token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| format!("Bearer {t}"))
}

async fn fetch_health(base: &str) -> Result<bool, String> {
    let health_url = format!("{base}/health");
    let resp = gloo_net::http::Request::get(&health_url)
        .send()
        .await
        .map_err(|e| format!("health fetch: {e}"))?;
    if !resp.ok() {
        return Err(format!("health HTTP {}", resp.status()));
    }
    let health: Health = resp
        .json()
        .await
        .map_err(|e| format!("health decode: {e}"))?;
    Ok(health.ok)
}

async fn fetch_status(base: &str, token: Option<&str>) -> Result<BoxStatus, String> {
    let status_url = format!("{base}/v1/status");
    let mut builder = gloo_net::http::Request::get(&status_url);
    if let Some(header) = auth_header(token) {
        builder = builder.header("Authorization", &header);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| format!("status fetch: {e}"))?;
    if !resp.ok() {
        return Err(format!("status HTTP {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("status decode: {e}"))
}

pub async fn fetch_snapshot(base_url: String, token: Option<String>) -> Snapshot {
    let base = base_url.trim_end_matches('/').to_string();
    let mut snap = Snapshot {
        health_ok: None,
        status: None,
        error: None,
    };
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
