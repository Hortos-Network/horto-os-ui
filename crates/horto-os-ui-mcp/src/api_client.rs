//! HTTP client for horto-os-ui-status-api day-2 routes.

use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::Value;

use crate::config::McpSettings;

const CONFIRM_BACKUP_ETC: &str = "backup-etc";

/// Thin client over status-api.
#[derive(Debug, Clone)]
pub struct StatusApiClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl StatusApiClient {
    /// Build from MCP settings.
    ///
    /// # Errors
    ///
    /// Returns when the HTTP client cannot be built.
    pub fn from_settings(settings: &McpSettings) -> Result<Self> {
        let base = settings.status_api_url.trim_end_matches('/').to_owned();
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            base,
            token: settings.api_token.clone(),
            http,
        })
    }

    fn auth_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.token {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .context("invalid HORTO_API_TOKEN for Authorization header")?;
            headers.insert(AUTHORIZATION, value);
        }
        Ok(headers)
    }

    /// `GET /health`.
    ///
    /// # Errors
    ///
    /// Returns on transport or non-success HTTP status.
    pub async fn health(&self) -> Result<Value> {
        let url = format!("{}/health", self.base);
        let res = self.http.get(&url).send().await.context("GET /health")?;
        let status = res.status();
        let body = res.text().await.context("read /health body")?;
        if !status.is_success() {
            bail!("GET /health -> {status}: {body}");
        }
        serde_json::from_str(&body).context("parse /health JSON")
    }

    /// `GET /v1/status`.
    ///
    /// # Errors
    ///
    /// Returns on transport or non-success HTTP status.
    pub async fn status(&self) -> Result<Value> {
        let url = format!("{}/v1/status", self.base);
        let res = self
            .http
            .get(&url)
            .headers(self.auth_headers()?)
            .send()
            .await
            .context("GET /v1/status")?;
        let status = res.status();
        let body = res.text().await.context("read /v1/status body")?;
        if !status.is_success() {
            bail!("GET /v1/status -> {status}: {body}");
        }
        serde_json::from_str(&body).context("parse /v1/status JSON")
    }

    /// Confirmed `POST /v1/backup/etc`.
    ///
    /// # Errors
    ///
    /// Returns when confirm is wrong, token missing, or the API rejects the call.
    pub async fn backup_etc(&self, confirm: &str) -> Result<Value> {
        if confirm != CONFIRM_BACKUP_ETC {
            bail!("confirm must be exactly `{CONFIRM_BACKUP_ETC}`");
        }
        if self.token.as_ref().is_none_or(|t| t.is_empty()) {
            bail!("HORTO_API_TOKEN required for backup_etc");
        }
        let url = format!("{}/v1/backup/etc", self.base);
        let mut headers = self.auth_headers()?;
        headers.insert(
            "X-Horto-Confirm",
            HeaderValue::from_static(CONFIRM_BACKUP_ETC),
        );
        let res = self
            .http
            .post(&url)
            .headers(headers)
            .send()
            .await
            .context("POST /v1/backup/etc")?;
        let status = res.status();
        let body = res.text().await.context("read backup body")?;
        if !status.is_success() {
            bail!("POST /v1/backup/etc -> {status}: {body}");
        }
        if body.trim().is_empty() {
            return Ok(serde_json::json!({ "ok": true }));
        }
        serde_json::from_str(&body).or_else(|_| Ok(serde_json::json!({ "ok": true, "raw": body })))
    }
}
