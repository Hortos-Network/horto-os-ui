//! Privileged ops via the shared OpenSSH remote runner (`HORTO_MCP_MODE=pc`).

use anyhow::{bail, Context, Result};
use horto_os_ui_shared::{
    remote_probe_arch, remote_run_cli, remote_setup_run, EcosystemInstallChoice, RemoteOptions,
    RemoteRunRequest, SystemProcessRunner,
};

use crate::config::McpSettings;

fn remote_options(settings: &McpSettings) -> Result<RemoteOptions> {
    let host = settings
        .remote_host
        .clone()
        .context("HORTO_REMOTE_HOST required for remote tools (OpenSSH Host alias or user@host)")?;
    let mut opts = RemoteOptions {
        host,
        install_ssh_key: settings.install_ssh_key,
        bin_dir: settings.bin_dir.clone(),
        force_askpass: true,
        ..RemoteOptions::default()
    };
    if let Some(tag) = &settings.release_tag {
        opts.release_tag = tag.clone();
    }
    Ok(opts)
}

fn run_cli(settings: &McpSettings, args: &[&str], use_sudo: bool) -> Result<String> {
    let opts = remote_options(settings)?;
    let cli_args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    remote_run_cli(
        &SystemProcessRunner,
        &RemoteRunRequest {
            options: opts,
            cli_args,
            use_sudo,
            install_payload_on_success: false,
            ecosystem: EcosystemInstallChoice::none(),
            offer_reboot_on_success: false,
            capture_output: false,
        },
    )
    .map(|o| o.log)
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Probe box arch over SSH.
///
/// # Errors
///
/// Returns when SSH or uname fails.
pub fn remote_probe(settings: &McpSettings) -> Result<String> {
    let opts = remote_options(settings)?;
    remote_probe_arch(&SystemProcessRunner, &opts)
        .map(|a| a.cache_label().to_owned())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Remote `setup status`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn setup_status(settings: &McpSettings, apply: bool, full: bool) -> Result<String> {
    let mut args = Vec::new();
    if apply {
        args.push("--apply");
    }
    args.push("setup");
    args.push("status");
    if full {
        args.push("--full");
    } else {
        args.push("--minimal");
    }
    run_cli(settings, &args, false)
}

/// Remote `setup run` (optional payload install when apply).
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn setup_run(
    settings: &McpSettings,
    apply: bool,
    full: bool,
    skip_piper: bool,
) -> Result<String> {
    let opts = remote_options(settings)?;
    remote_setup_run(
        &SystemProcessRunner,
        opts,
        apply,
        full,
        skip_piper,
        EcosystemInstallChoice::none(),
    )
    .map(|o| o.log)
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Remote `setup step`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn setup_step(
    settings: &McpSettings,
    step_id: &str,
    apply: bool,
    full: bool,
) -> Result<String> {
    let kind = if full { "full" } else { "minimal" };
    let mut args = Vec::new();
    if apply {
        args.push("--apply");
    }
    args.push("setup");
    args.push("step");
    args.push(step_id);
    args.push(kind);
    run_cli(settings, &args, apply)
}

/// Remote `doctor`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn doctor(settings: &McpSettings) -> Result<String> {
    run_cli(settings, &["doctor"], false)
}

/// Remote `docker status`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn docker_status(settings: &McpSettings) -> Result<String> {
    run_cli(settings, &["docker", "status"], false)
}

/// Remote `docker rebuild` (confirm required by caller).
///
/// # Errors
///
/// Returns when confirm is wrong or remote fails.
pub fn docker_rebuild(settings: &McpSettings, confirm: &str) -> Result<String> {
    if confirm != "docker-rebuild" {
        bail!("confirm must be exactly `docker-rebuild`");
    }
    run_cli(settings, &["docker", "rebuild"], true)
}

/// Remote `backup list`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn backup_list(settings: &McpSettings) -> Result<String> {
    run_cli(settings, &["backup", "list"], false)
}

/// Remote `backup disk-status`.
///
/// # Errors
///
/// Returns on remote runner failure.
pub fn backup_disk_status(settings: &McpSettings) -> Result<String> {
    run_cli(settings, &["backup", "disk-status"], false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_rebuild_rejects_bad_confirm() {
        let settings = McpSettings {
            mode: crate::config::McpMode::Pc,
            status_api_url: "http://127.0.0.1:8787".into(),
            api_token: None,
            mcp_token: None,
            remote_host: Some("horto".into()),
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        let err = docker_rebuild(&settings, "nope").expect_err("confirm");
        assert!(err.to_string().contains("docker-rebuild"));
    }

    #[test]
    fn remote_options_require_host() {
        let settings = McpSettings {
            mode: crate::config::McpMode::Pc,
            status_api_url: "http://127.0.0.1:8787".into(),
            api_token: None,
            mcp_token: None,
            remote_host: None,
            release_tag: None,
            bin_dir: None,
            install_ssh_key: false,
        };
        assert!(remote_options(&settings).is_err());
    }
}
