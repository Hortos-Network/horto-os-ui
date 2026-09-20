//! Privileged ops in-process when MCP runs on the box (`HORTO_MCP_MODE=box`).

use anyhow::{bail, Result};
use horto_os_ui_shared::{
    backup_status, docker_rebuild, doctor, list_containers, list_timestamped_etc_backups,
    setup_run, setup_status as engine_setup_status, setup_step, ApplyMode, HostContext,
    NonInteractivePrompts, SetupKind,
};

const CONFIRM_DOCKER_REBUILD: &str = "docker-rebuild";

fn ctx(dry_run: bool) -> HostContext {
    let mode = if dry_run {
        ApplyMode::DryRun
    } else {
        ApplyMode::Apply
    };
    HostContext::new(mode, SetupKind::Full).with_prompts(Box::new(NonInteractivePrompts))
}

fn kind(full: bool) -> SetupKind {
    if full {
        SetupKind::Full
    } else {
        SetupKind::Minimal
    }
}

fn capture_logs(host: &HostContext) -> String {
    let mut out = host.logs.join("\n");
    if !host.planned.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        for p in &host.planned {
            out.push_str("[plan] ");
            out.push_str(&p.summary);
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        out = "ok".into();
    }
    out
}

/// Embedded setup status (resume report).
///
/// # Errors
///
/// Returns on serialize failure.
pub fn setup_status_report(dry_run: bool, full: bool) -> Result<String> {
    let c = ctx(dry_run);
    let report = engine_setup_status(&c, kind(full));
    Ok(serde_json::to_string_pretty(&report)?)
}

/// Embedded `setup run`.
///
/// # Errors
///
/// Returns engine errors (root required on apply, step failures).
pub fn setup_run_embedded(dry_run: bool, full: bool, skip_piper: bool) -> Result<String> {
    let mut c = ctx(dry_run);
    c.skip_piper = skip_piper;
    setup_run(&mut c, kind(full)).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(capture_logs(&c))
}

/// Embedded `setup step`.
///
/// # Errors
///
/// Returns engine errors.
pub fn setup_step_embedded(step_id: &str, dry_run: bool, full: bool) -> Result<String> {
    let mut c = ctx(dry_run);
    setup_step(&mut c, kind(full), step_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(capture_logs(&c))
}

/// Embedded doctor.
///
/// # Errors
///
/// Returns on serialize failure.
pub fn doctor_embedded() -> Result<String> {
    let c = ctx(true);
    let report = doctor(&c);
    Ok(serde_json::to_string_pretty(&report)?)
}

/// Embedded docker status.
///
/// # Errors
///
/// Returns when listing containers fails.
pub fn docker_status_embedded() -> Result<String> {
    let list = list_containers().map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(serde_json::to_string_pretty(&list)?)
}

/// Embedded docker rebuild.
///
/// # Errors
///
/// Returns when confirm is wrong or rebuild fails.
pub fn docker_rebuild_embedded(confirm: &str) -> Result<String> {
    if confirm != CONFIRM_DOCKER_REBUILD {
        bail!("confirm must be exactly `{CONFIRM_DOCKER_REBUILD}`");
    }
    let assets = std::path::Path::new("/srv/docker");
    docker_rebuild(assets).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok("docker rebuild finished".into())
}

/// List timestamped etc backups.
///
/// # Errors
///
/// Returns on serialize failure.
pub fn backup_list_embedded() -> Result<String> {
    let c = ctx(true);
    let list = list_timestamped_etc_backups(&c);
    Ok(serde_json::to_string_pretty(&list)?)
}

/// Disk backup probe / status.
///
/// # Errors
///
/// Returns on serialize failure.
pub fn backup_disk_status_embedded() -> Result<String> {
    let c = ctx(true);
    let status = backup_status(&c);
    Ok(serde_json::to_string_pretty(&status)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_rebuild_rejects_bad_confirm() {
        let err = docker_rebuild_embedded("nope").expect_err("confirm");
        assert!(err.to_string().contains("docker-rebuild"));
    }

    #[test]
    fn setup_status_dry_run_serializes() {
        let text = setup_status_report(true, true).expect("status");
        assert!(text.contains('{') || text.contains("steps") || !text.is_empty());
    }
}
