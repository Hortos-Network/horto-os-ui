//! Apt / dpkg helpers that respect [`HostContext`] dry-run vs apply.

use crate::context::HostContext;
use crate::error::{HortoError, Result};
use std::process::Command;

/// Run `apt update`.
///
/// In dry-run mode, records a planned action and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `apt` cannot be spawned or exits non-zero.
pub fn apt_update(ctx: &mut HostContext) -> Result<()> {
    run_apt(ctx, &["update"])
}

/// Run `apt install -y` for `packages`.
///
/// In dry-run mode, records a planned action and returns `Ok`.
///
/// # Errors
///
/// Returns [`HortoError::CommandFailed`] when `apt` cannot be spawned or exits non-zero.
pub fn apt_install(ctx: &mut HostContext, packages: &[&str]) -> Result<()> {
    let mut args = vec!["install", "-y"];
    args.extend_from_slice(packages);
    run_apt(ctx, &args)
}

fn run_apt(ctx: &mut HostContext, args: &[&str]) -> Result<()> {
    let summary = format!("apt {}", args.join(" "));
    if ctx.is_dry_run() {
        ctx.plan_action(summary);
        return Ok(());
    }
    ctx.log(&summary);
    let status = Command::new("apt")
        .args(args)
        .status()
        .map_err(|e| HortoError::command("apt", e.to_string()))?;
    if !status.success() {
        return Err(HortoError::command(
            "apt",
            format!("{} -> {status}", args.join(" ")),
        ));
    }
    Ok(())
}

/// True when `dpkg-query` reports `install ok installed` for `name`.
#[must_use]
pub fn package_installed(name: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", name])
        .output()
        .is_ok_and(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains("install ok installed")
        })
}
