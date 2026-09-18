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

pub fn stop(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["stop", unit])
}

pub fn disable(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["disable", unit])
}

pub fn enable(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["enable", unit])
}

pub fn start(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["start", unit])
}

pub fn restart(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["restart", unit])
}

pub fn unmask(ctx: &mut HostContext, unit: &str) -> Result<()> {
    systemctl(ctx, &["unmask", unit])
}

/// Best-effort variants that log warnings instead of failing.
pub fn try_restart(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = restart(ctx, unit) {
        ctx.log(format!("warning: restart {unit}: {e}"));
    }
}

pub fn try_enable(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = enable(ctx, unit) {
        ctx.log(format!("warning: enable {unit}: {e}"));
    }
}

pub fn try_unmask(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = unmask(ctx, unit) {
        ctx.log(format!("warning: unmask {unit}: {e}"));
    }
}

pub fn try_start(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = start(ctx, unit) {
        ctx.log(format!("warning: start {unit}: {e}"));
    }
}

pub fn try_stop(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = stop(ctx, unit) {
        ctx.log(format!("warning: stop {unit}: {e}"));
    }
}

pub fn try_disable(ctx: &mut HostContext, unit: &str) {
    if let Err(e) = disable(ctx, unit) {
        ctx.log(format!("warning: disable {unit}: {e}"));
    }
}

pub fn unit_present(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["list-unit-files", &format!("{unit}.service")])
        .output()
        .ok()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}
