//! Ensure Docker Engine is installed before stack files (d1) and compose up (d2).
//!
//! Prefer Armbian `armbian-config --cmd CON001` when available; otherwise apt
//! packages `docker.io` and `docker-compose-v2`.

use crate::context::{is_root, HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::{apt, docker};
use crate::step::Step;
use std::process::Command;

pub struct D0DockerEngine;

const APT_PACKAGES: &[&str] = &["docker.io", "docker-compose-v2"];

impl Step for D0DockerEngine {
    fn id(&self) -> &'static str {
        "d0"
    }
    fn title(&self) -> &'static str {
        "Install Docker Engine"
    }
    fn reference_script(&self) -> &'static str {
        "d0_docker_engine.sh"
    }
    fn step_version(&self) -> u32 {
        1
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &[]
    }
    fn is_done(&self, _ctx: &HostContext) -> bool {
        docker_engine_ready()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        if docker_engine_ready() {
            ctx.plan_action("docker engine already available (docker info ok)");
        } else if which::which("armbian-config").is_ok() {
            ctx.plan_action("armbian-config --cmd CON001 (install Docker Engine)");
        } else {
            ctx.plan_action("apt update");
            ctx.plan_action(format!("apt install -y {}", APT_PACKAGES.join(" ")));
            ctx.plan_action("systemctl enable --now docker");
        }
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if docker_engine_ready() {
            ctx.log("Docker Engine already available.");
            return Ok(());
        }
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        if !is_root() {
            return Err(HortoError::msg(
                "d0 requires root to install Docker Engine (or install Docker first)",
            ));
        }
        install_docker_engine(ctx)?;
        if !docker_engine_ready() {
            return Err(HortoError::msg(
                "Docker Engine still missing after install. On Armbian run: sudo armbian-config --cmd CON001",
            ));
        }
        ctx.log("Step d0 complete: Docker Engine ready.");
        Ok(())
    }
}

/// True when `docker` is on PATH and `docker info` succeeds.
#[must_use]
pub fn docker_engine_ready() -> bool {
    if !docker::docker_available() {
        return false;
    }
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn install_docker_engine(ctx: &mut HostContext) -> Result<()> {
    if which::which("armbian-config").is_ok() {
        ctx.log("armbian-config --cmd CON001");
        let status = Command::new("armbian-config")
            .args(["--cmd", "CON001"])
            .status()
            .map_err(|e| HortoError::command("armbian-config", e.to_string()))?;
        if status.success() && docker_engine_ready() {
            return Ok(());
        }
        ctx.log("armbian-config CON001 did not leave docker ready; falling back to apt packages");
    }
    apt::apt_update(ctx)?;
    apt::apt_install(ctx, APT_PACKAGES)?;
    enable_docker_service(ctx)?;
    Ok(())
}

fn enable_docker_service(ctx: &mut HostContext) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action("systemctl enable --now docker");
        return Ok(());
    }
    ctx.log("systemctl enable --now docker");
    let status = Command::new("systemctl")
        .args(["enable", "--now", "docker"])
        .status()
        .map_err(|e| HortoError::command("systemctl", e.to_string()))?;
    if !status.success() {
        return Err(HortoError::command(
            "systemctl",
            format!("enable --now docker -> {status}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApplyMode;
    use crate::pipeline::SetupKind;

    #[test]
    fn metadata_is_stable() {
        let step = D0DockerEngine;
        assert_eq!(step.id(), "d0");
        assert_eq!(step.reference_script(), "d0_docker_engine.sh");
        assert_eq!(step.step_version(), 1);
        assert!(step.depends_on().is_empty());
        assert!(!step.title().is_empty());
    }

    #[test]
    fn dry_run_plans_without_install() {
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        D0DockerEngine.apply(&mut ctx).unwrap();
        assert!(
            ctx.planned.iter().any(|p| p.summary.contains("docker"))
                || ctx.logs.iter().any(|l| l.contains("already available"))
        );
    }
}
