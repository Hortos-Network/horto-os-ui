//! `systemctl` helpers that respect [`HostContext`] dry-run vs apply.

use crate::context::HostContext;
use crate::error::{HortoError, Result};
use std::process::Command;

fn systemctl(ctx: &mut HostContext, args: &[&str]) -> Result<()> {
    let summary = format!("systemctl {}", args.join(" "));
    if ctx.is_dry_run() {
        ctx.plan_action(summary);
        return Ok(());
    }
    ctx.log(&summary);
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .map_err(|e| HortoError::command("systemctl", e.to_string()))?;
    if !status.success() {
        return Err(HortoError::command(
            "systemctl",
            format!("{} -> {status}", args.join(" ")),
        ));
    }
    Ok(())
}

/// Stop a systemd unit.
///
/// In dry-run mode, records `systemctl stop` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn stop(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["stop", unit])
}

/// Disable a systemd unit.
///
/// In dry-run mode, records `systemctl disable` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn disable(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["disable", unit])
}

/// Enable a systemd unit.
///
/// In dry-run mode, records `systemctl enable` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn enable(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["enable", unit])
}

/// Start a systemd unit.
///
/// In dry-run mode, records `systemctl start` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn start(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["start", unit])
}

/// Restart a systemd unit.
///
/// In dry-run mode, records `systemctl restart` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn restart(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["restart", unit])
}

/// Unmask a systemd unit.
///
/// In dry-run mode, records `systemctl unmask` and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` cannot be spawned or exits non-zero.
pub fn unmask(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["unmask", unit])
}

/// Reload systemd unit files (`daemon-reload`).
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` fails.
pub fn daemon_reload(ctx: &mut HostContext) -> Result<()> {
    systemctl(ctx, &["daemon-reload"])
}

/// Enable and start a unit in one shot (`enable --now`).
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `systemctl` fails.
pub fn enable_now(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["enable", "--now", unit])
}

/// Restart `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_restart(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = restart(ctx, unit) {
        ctx.log(format!("warning: restart {unit}: {e}"));
    }
}

/// `daemon-reload`; on failure append a warning.
pub fn try_daemon_reload(ctx: &mut HostContext) {
    if let Err(e) = daemon_reload(ctx) {
        ctx.log(format!("warning: daemon-reload: {e}"));
    }
}

/// Enable `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_enable(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = enable(ctx, unit) {
        ctx.log(format!("warning: enable {unit}: {e}"));
    }
}

/// Unmask `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_unmask(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = unmask(ctx, unit) {
        ctx.log(format!("warning: unmask {unit}: {e}"));
    }
}

/// Start `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_start(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = start(ctx, unit) {
        ctx.log(format!("warning: start {unit}: {e}"));
    }
}

/// Stop `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_stop(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = stop(ctx, unit) {
        ctx.log(format!("warning: stop {unit}: {e}"));
    }
}

/// Disable `unit`; on failure append a warning to [`HostContext::logs`].
pub fn try_disable(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = disable(ctx, unit) {
        ctx.log(format!("warning: disable {unit}: {e}"));
    }
}

/// True when `systemctl list-unit-files` reports `{unit}.service`.
#[must_use]
pub fn unit_present(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["list-unit-files", &format!("{unit}.service")])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}
