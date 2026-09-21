//! Tauri commands for remote OpenSSH setup (Desktop is always remote).

use horto_os_ui_shared::{
    format_surfaces_report, probe_surfaces, remote_probe_arch, remote_setup_run,
    EcosystemInstallChoice, RemoteOptions, RemoteOptionsInput, SurfaceProbeReport,
    SystemProcessRunner,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Arguments for a remote setup run from the Desktop UI.
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
    /// GitHub Release tag (`v0.1.0` or `dev-preview`).
    #[serde(default)]
    pub release_tag: Option<String>,
    /// When true, apply remote setup (default false = plan only).
    #[serde(default)]
    pub apply: bool,
    /// Full pipeline when true; minimal when false.
    #[serde(default = "default_true")]
    pub full: bool,
    /// Skip piper download on the box.
    #[serde(default)]
    pub skip_piper: bool,
    /// Opt-in: install status-api systemd unit after apply.
    #[serde(default)]
    pub install_status_api: bool,
    /// Opt-in: install MCP systemd unit after apply.
    #[serde(default)]
    pub install_mcp: bool,
}

fn default_true() -> bool {
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
    if release_tag
        .as_ref()
        .map(|t| t.trim().is_empty())
        .unwrap_or(true)
    {
        if let Ok(env_tag) = std::env::var("HORTO_RELEASE_TAG") {
            let trimmed = env_tag.trim();
            if !trimmed.is_empty() {
                release_tag = Some(trimmed.to_owned());
            }
        }
    }
    RemoteOptions::from_input(RemoteOptionsInput {
        host: args.host.clone(),
        install_ssh_key: args.install_ssh_key,
        bin_dir: args.bin_dir.as_ref().map(PathBuf::from),
        release_tag,
        force_askpass: true,
    })
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
pub fn sync_api_token_from_box(host: String) -> Result<String, String> {
    let host = host.trim().to_owned();
    if host.is_empty() {
        return Err("Set a host first.".into());
    }
    let opts = RemoteOptions::from_input(RemoteOptionsInput {
        host,
        force_askpass: true,
        ..RemoteOptionsInput::default()
    });
    horto_os_ui_shared::pull_remote_api_token(&SystemProcessRunner, &opts)
        .map_err(|e| e.to_string())
}

/// Probe box architecture over SSH.
#[tauri::command]
pub fn remote_probe(host: String) -> Result<String, String> {
    let opts = RemoteOptions::from_input(RemoteOptionsInput {
        host,
        force_askpass: true,
        ..RemoteOptionsInput::default()
    });
    remote_probe_arch(&SystemProcessRunner, &opts)
        .map(|a| a.cache_label().to_owned())
        .map_err(|e| e.to_string())
}

/// Probe SSH / CLI / API / MCP surfaces (same report as CLI `surfaces` / TUI).
#[tauri::command]
pub fn remote_surfaces_probe(host: String) -> Result<SurfaceProbeReport, String> {
    let host = host.trim().to_owned();
    if host.is_empty() {
        return Err("Set an OpenSSH Host alias or user@host first.".into());
    }
    let opts = RemoteOptions::from_input(RemoteOptionsInput {
        host,
        force_askpass: true,
        ..RemoteOptionsInput::default()
    });
    probe_surfaces(&SystemProcessRunner, &opts, false).map_err(|e| e.to_string())
}

/// Human text for Connection UI (same formatter as CLI).
#[tauri::command]
pub fn remote_surfaces_text(host: String) -> Result<String, String> {
    let report = remote_surfaces_probe(host)?;
    Ok(format_surfaces_report(&report))
}

/// Run remote setup (uploads CLI agent, optional key install, `setup run`).
///
/// Returns `{ log, apiToken? }` for Connection propose-save.
#[tauri::command]
pub fn remote_setup(args: RemoteSetupArgs) -> Result<RemoteSetupResult, String> {
    let opts = options_from(&args);
    let ecosystem = EcosystemInstallChoice {
        status_api: args.install_status_api,
        mcp: args.install_mcp,
    };
    let outcome = remote_setup_run(
        &SystemProcessRunner,
        opts,
        args.apply,
        args.full,
        args.skip_piper,
        if args.apply {
            ecosystem
        } else {
            EcosystemInstallChoice::none()
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(RemoteSetupResult {
        log: outcome.log,
        api_token: outcome.api_token,
    })
}
