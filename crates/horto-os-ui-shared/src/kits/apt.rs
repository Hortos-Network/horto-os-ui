use crate::context::HostContext;
use crate::error::{HortoError, Result};
use std::process::Command;

pub fn apt_update(ctx: &mut HostContext) -> Result<()> {
    run_apt(ctx, &["update"])
}

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

pub fn package_installed(name: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", name])
        .output()
        .ok()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains("install ok installed")
        })
        .unwrap_or(false)
}
