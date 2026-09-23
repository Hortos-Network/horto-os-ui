//! Tauri commands for remote OpenSSH setup (Desktop is always remote).
//!
//! SSH / setup work runs on a blocking pool so the webview can keep animating.

use horto_os_ui_shared::{
    format_surfaces_report, list_known_remote_hosts, probe_surfaces, remote_probe_arch,
    remote_setup_run, remote_upload_cli, EcosystemInstallChoice, KnownRemoteHost, RemoteCliProbe,
    RemoteOptions, RemoteOptionsInput, RemoteSetupRunArgs, StackOpts, SurfaceProbeReport,
    SystemProcessRunner, DEFAULT_GITHUB_REPO, LONG_VERSION,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tip Pre-release tag used when Desktop has no explicit release tag / env.
const DESKTOP_TIP_RELEASE_TAG: &str = "dev-preview";

/// Arguments for a remote setup run from the Desktop UI.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSetupArgs {
    /// OpenSSH Host alias or `user@host`.
    pub host: String,
    /// Opt-in key install. Default false when omitted.
    #[serde(default)]
    pub install_ssh_key: bool,
    /// Local bin dir override.
    #[serde(default)]
    pub bin_dir: Option<String>,
    /// GitHub Release tag (`v0.1.0` or tip `dev-preview`).
    #[serde(default)]
    pub release_tag: Option<String>,
    /// When true, apply remote setup (default false = plan only).
    #[serde(default)]
    pub apply: bool,
    /// Full setup when true; minimal when false.
    #[serde(default = "default_true")]
    pub full: bool,
    /// Skip piper download on the box.
    #[serde(default)]
    pub skip_piper: bool,
    /// Install status-api systemd unit after apply (default on).
    #[serde(default = "default_true")]
    pub install_status_api: bool,
    /// Install MCP systemd unit after apply (default on).
    #[serde(default = "default_true")]
    pub install_mcp: bool,
    /// Extra Docker apps CSV (`dockge,open-webui,…`).
    #[serde(default)]
    pub stacks: String,
    /// When true, allow setup even if box CLI long-version ≠ tip. Default false.
    #[serde(default)]
    pub allow_stale_cli: bool,
}

const fn default_true() -> bool {
    true
}

/// JSON payload returned to Connection after remote setup.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSetupResult {
    pub log: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
}

fn options_from(args: &RemoteSetupArgs) -> RemoteOptions {
    let mut release_tag = args.release_tag.clone();
    if release_tag.as_ref().is_none_or(|t| t.trim().is_empty()) {
        if let Ok(env_tag) = std::env::var("HORTO_RELEASE_TAG") {
            let trimmed = env_tag.trim();
            if !trimmed.is_empty() {
                release_tag = Some(trimmed.to_owned());
            }
        }
    }
    if release_tag.as_ref().is_none_or(|t| t.trim().is_empty()) {
        // Only tip Pre-release exists until a stable v* cut; match Make RELEASE_TAG.
        release_tag = Some(DESKTOP_TIP_RELEASE_TAG.to_owned());
    }
    let mut bin_dir = args.bin_dir.as_ref().map(PathBuf::from);
    if bin_dir.is_none() {
        if let Ok(env_dir) = std::env::var("HORTO_BIN_DIR") {
            let trimmed = env_dir.trim();
            if !trimmed.is_empty() {
                bin_dir = Some(PathBuf::from(trimmed));
            }
        }
    }
    RemoteOptions::from_input(RemoteOptionsInput {
        host: args.host.clone(),
        install_ssh_key: args.install_ssh_key,
        bin_dir,
        release_tag,
        force_askpass: true,
    })
}

async fn blocking_err<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

/// Read the shared tip bearer (`api_token` under the horto-os-ui config dir).
///
/// Empty string means no usable tip file (avoids `Option` IPC ambiguity in the webview).
#[tauri::command]
pub fn read_api_token() -> Result<String, String> {
    Ok(horto_os_ui_shared::read_local_api_token().unwrap_or_default())
}

/// Write the shared tip bearer (same file as TUI / CLI).
#[tauri::command]
pub fn write_api_token(token: String) -> Result<(), String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    horto_os_ui_shared::write_api_token_file(trimmed).map_err(|e| e.to_string())?;
    Ok(())
}

/// Pull the box bearer over SSH (sudo) into the tip file and return it.
#[tauri::command]
pub async fn sync_api_token_from_box(host: String) -> Result<String, String> {
    blocking_err(move || {
        let host = host.trim().to_owned();
        if host.is_empty() {
            return Err("Pick or enter a host first.".into());
        }
        let opts = RemoteOptions::from_input(RemoteOptionsInput {
            host,
            force_askpass: true,
            ..RemoteOptionsInput::default()
        });
        horto_os_ui_shared::pull_remote_api_token(&SystemProcessRunner, &opts)
            .map_err(|e| e.to_string())
    })
    .await
}

/// List LAN `/etc/hosts` names and LAN-qualified OpenSSH aliases.
#[tauri::command]
pub async fn list_known_remote_hosts_cmd() -> Result<Vec<KnownRemoteHost>, String> {
    blocking_err(|| list_known_remote_hosts().map_err(|e| e.to_string())).await
}

/// List Release tags from GitHub (`DEFAULT_GITHUB_REPO`), tip first when present.
#[tauri::command]
pub async fn list_release_tags_cmd() -> Result<Vec<String>, String> {
    blocking_err(list_github_release_tags).await
}

fn list_github_release_tags() -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{DEFAULT_GITHUB_REPO}/releases?per_page=30");
    let body = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "horto-os-ui-desktop")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| format!("GitHub releases request failed: {e}"))?
        .into_string()
        .map_err(|e| format!("GitHub releases body failed: {e}"))?;
    let releases: Vec<serde_json::Value> =
        serde_json::from_str(&body).map_err(|e| format!("GitHub releases JSON failed: {e}"))?;
    let mut tags: Vec<String> = releases
        .iter()
        .filter_map(|r| {
            r.get("tag_name")
                .and_then(|t| t.as_str())
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect();
    tags.dedup();
    if let Some(i) = tags.iter().position(|t| t == DESKTOP_TIP_RELEASE_TAG) {
        let tip = tags.remove(i);
        tags.insert(0, tip);
    }
    if tags.is_empty() {
        tags.push(DESKTOP_TIP_RELEASE_TAG.to_owned());
    }
    Ok(tags)
}

/// Probe box architecture over SSH.
#[tauri::command]
pub async fn remote_probe(host: String) -> Result<String, String> {
    blocking_err(move || {
        let opts = RemoteOptions::from_input(RemoteOptionsInput {
            host,
            force_askpass: true,
            ..RemoteOptionsInput::default()
        });
        remote_probe_arch(&SystemProcessRunner, &opts)
            .map(|a| a.cache_label().to_owned())
            .map_err(|e| e.to_string())
    })
    .await
}

/// Probe SSH / CLI / API / MCP surfaces (same report as CLI `surfaces` / TUI).
#[tauri::command]
pub async fn remote_surfaces_probe(host: String) -> Result<SurfaceProbeReport, String> {
    blocking_err(move || {
        let host = host.trim().to_owned();
        if host.is_empty() {
            return Err("Pick or enter a host first.".into());
        }
        let opts = RemoteOptions::from_input(RemoteOptionsInput {
            host,
            force_askpass: true,
            ..RemoteOptionsInput::default()
        });
        probe_surfaces(&SystemProcessRunner, &opts, false).map_err(|e| e.to_string())
    })
    .await
}

/// Human text for Connection UI (same formatter as CLI).
#[tauri::command]
pub async fn remote_surfaces_text(host: String) -> Result<String, String> {
    let report = remote_surfaces_probe(host).await?;
    Ok(format_surfaces_report(&report))
}

/// Tip long-version baked into this Desktop binary (`0.1.0 (abc1234)`).
#[tauri::command]
pub fn tip_cli_version() -> String {
    LONG_VERSION.to_owned()
}

/// Upload tip CLI agent to the box (s0 / Sync CLI). Returns probe after SCP.
#[tauri::command]
pub async fn remote_upload_cli_cmd(args: RemoteSetupArgs) -> Result<RemoteCliProbeUi, String> {
    blocking_err(move || {
        let opts = options_from(&args);
        let probe = remote_upload_cli(&SystemProcessRunner, &opts).map_err(|e| e.to_string())?;
        Ok(RemoteCliProbeUi::from(&probe))
    })
    .await
}

/// Compact CLI probe for Connection UI after sync / for display.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCliProbeUi {
    /// Normalized box version when found.
    pub version: Option<String>,
    /// True when box matches tip [`LONG_VERSION`].
    pub current: bool,
    /// Operator label (`missing`, version, …).
    pub label: String,
    /// Tip long-version this Desktop expects.
    pub tip_version: String,
}

impl From<&RemoteCliProbe> for RemoteCliProbeUi {
    fn from(probe: &RemoteCliProbe) -> Self {
        Self {
            version: probe.version.clone(),
            current: probe.current,
            label: probe.status.as_label().to_owned(),
            tip_version: LONG_VERSION.to_owned(),
        }
    }
}

/// Run remote setup (uploads CLI agent, optional key install, `setup run`).
///
/// Returns `{ log, apiToken? }` for Connection propose-save.
#[tauri::command]
pub async fn remote_setup(args: RemoteSetupArgs) -> Result<RemoteSetupResult, String> {
    blocking_err(move || {
        let allow_stale = args.allow_stale_cli;
        let opts = options_from(&args);
        let ecosystem = EcosystemInstallChoice {
            status_api: args.install_status_api,
            mcp: args.install_mcp,
        };
        let stack_opts = StackOpts::parse_csv(&args.stacks);
        let outcome = remote_setup_run(
            &SystemProcessRunner,
            RemoteSetupRunArgs {
                options: opts,
                apply: args.apply,
                full: args.full,
                skip_piper: args.skip_piper,
                ecosystem: if args.apply {
                    ecosystem
                } else {
                    EcosystemInstallChoice::none()
                },
                stack_opts,
                allow_stale_cli: allow_stale,
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(RemoteSetupResult {
            log: outcome.log,
            api_token: outcome.api_token,
        })
    })
    .await
}
