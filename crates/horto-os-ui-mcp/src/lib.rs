//! Horto OS UI MCP library: status-api client, backends, and rmcp tools.

pub mod api_client;
pub mod config;
pub mod embedded_ops;
pub mod net;
pub mod remote_ops;
pub mod server;
pub mod tool_args;

pub use config::{McpMode, McpSettings};
pub use server::{run_http, HortoMcp, DEFAULT_HTTP_LISTEN_BOX, DEFAULT_HTTP_LISTEN_PC};

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::api_client::StatusApiClient;
    use crate::config::{McpMode, McpSettings};
    use crate::server::{default_listen_for, HortoMcp};
    use rmcp::ServerHandler;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn restore_env(key: &str, prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn mode_and_defaults() {
        let _g = env_lock();
        let keys = [
            "HORTO_MCP_MODE",
            "HORTO_STATUS_API_URL",
            "HORTO_API_TOKEN",
            "HORTO_MCP_TOKEN",
            "HORTO_REMOTE_HOST",
            "HORTO_RELEASE_TAG",
            "HORTO_BIN_DIR",
            "HORTO_INSTALL_SSH_KEY",
        ];
        let prev: Vec<_> = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        for k in keys {
            std::env::remove_var(k);
        }

        assert_eq!(McpMode::from_env(), McpMode::Pc);
        let pc = McpSettings::from_env();
        assert_eq!(pc.mode, McpMode::Pc);
        assert!(pc.status_api_url.contains("8787"));
        assert!(pc.http_bearer().is_none());
        assert_eq!(default_listen_for(McpMode::Pc), "127.0.0.1:8790");
        assert_eq!(default_listen_for(McpMode::Box), "0.0.0.0:8790");

        std::env::set_var("HORTO_MCP_MODE", "box");
        std::env::set_var("HORTO_API_TOKEN", "tok");
        let box_s = McpSettings::from_env();
        assert_eq!(box_s.mode, McpMode::Box);
        assert_eq!(box_s.http_bearer(), Some("tok"));
        assert!(box_s.status_api_url.contains("127.0.0.1"));

        std::env::set_var("HORTO_MCP_TOKEN", "mcp-only");
        let mcp = McpSettings::from_env();
        assert_eq!(mcp.http_bearer(), Some("mcp-only"));

        for (k, v) in prev {
            restore_env(k, v);
        }
    }

    #[test]
    fn server_info_identity() {
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
        let info = HortoMcp::new(settings).get_info();
        assert_eq!(info.server_info.name.as_str(), "horto-os-ui");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.capabilities.tools.is_some());
    }

    #[tokio::test]
    async fn api_client_health_status_backup() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/status"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hostname": "box"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/backup/etc"))
            .and(header("authorization", "Bearer secret"))
            .and(header("x-horto-confirm", "backup-etc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"path": "/backup"})))
            .mount(&server)
            .await;

        let settings = McpSettings {
            mode: McpMode::Pc,
            status_api_url: server.uri(),
            api_token: Some("secret".into()),
            mcp_token: Some("secret".into()),
            remote_host: None,
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        let client = StatusApiClient::from_settings(&settings).expect("client");
        let health = client.health().await.expect("health");
        assert_eq!(health["ok"], true);
        let status = client.status().await.expect("status");
        assert_eq!(status["hostname"], "box");
        let backup = client.backup_etc("backup-etc").await.expect("backup");
        assert_eq!(backup["path"], "/backup");
        assert!(client.backup_etc("nope").await.is_err());
    }

    #[tokio::test]
    async fn backup_requires_token() {
        let settings = McpSettings {
            mode: McpMode::Pc,
            status_api_url: "http://127.0.0.1:9".into(),
            api_token: None,
            mcp_token: None,
            remote_host: None,
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        let client = StatusApiClient::from_settings(&settings).expect("client");
        assert!(client.backup_etc("backup-etc").await.is_err());
    }
}
