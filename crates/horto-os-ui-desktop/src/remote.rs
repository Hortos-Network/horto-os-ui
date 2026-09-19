//! Tauri commands for remote OpenSSH setup (Desktop is always remote).

use horto_os_ui_shared::{remote_probe_arch, remote_setup_run, RemoteOptions, SystemProcessRunner};
use serde::Deserialize;
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
    /// Dry-run the remote setup pipeline.
    #[serde(default)]
    pub dry_run: bool,
    /// Full pipeline when true; minimal when false.
    #[serde(default = "default_true")]
    pub full: bool,
    /// Skip piper download on the box.
    #[serde(default)]
    pub skip_piper: bool,
}

fn default_true() -> bool {
    true
}

fn options_from(args: &RemoteSetupArgs) -> RemoteOptions {
    let mut opts = RemoteOptions {
        host: args.host.clone(),
        install_ssh_key: args.install_ssh_key,
        bin_dir: args.bin_dir.as_ref().map(PathBuf::from),
        force_askpass: true,
        ..RemoteOptions::default()
    };
    if let Some(tag) = args
        .release_tag
        .as_ref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
    {
        opts.release_tag = tag.to_owned();
    } else if let Ok(env_tag) = std::env::var("HORTO_RELEASE_TAG") {
        let trimmed = env_tag.trim();
        if !trimmed.is_empty() {
            opts.release_tag = trimmed.to_owned();
        }
    }
    opts
}

/// Probe box architecture over SSH.
#[tauri::command]
pub fn remote_probe(host: String) -> Result<String, String> {
    let opts = RemoteOptions {
        host,
        force_askpass: true,
        ..RemoteOptions::default()
    };
    remote_probe_arch(&SystemProcessRunner, &opts)
        .map(|a| a.cache_label().to_owned())
        .map_err(|e| e.to_string())
}

/// Run remote setup (uploads CLI agent, optional key install, `setup run`).
#[tauri::command]
pub fn remote_setup(args: RemoteSetupArgs) -> Result<String, String> {
    let opts = options_from(&args);
    remote_setup_run(
        &SystemProcessRunner,
        opts,
        args.dry_run,
        args.full,
        args.skip_piper,
        !args.dry_run,
    )
    .map_err(|e| e.to_string())
}
