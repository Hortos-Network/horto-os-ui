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
        plan_install_actions(ctx);
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if docker_engine_ready() {
            ctx.log("Docker Engine already available.");
            return Ok(());
        }
        if ctx.is_dry_run() {
            plan_install_actions(ctx);
            return Ok(());
        }
        if !is_root() {
            return Err(HortoError::msg(
                "d0 requires root to install Docker Engine (or install Docker first)",
            ));
        }
        install_docker_engine(ctx)?;
        finish_after_install(ctx)
    }
}

/// True when `docker` is on PATH and `docker info` succeeds.
#[must_use]
pub fn docker_engine_ready() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = ready_override() {
            return v;
        }
    }
    if !docker::docker_available() {
        return false;
    }
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn plan_install_actions(ctx: &mut HostContext) {
    if docker_engine_ready() {
        ctx.plan_action("docker engine already available (docker info ok)");
    } else if prefer_armbian_config() {
        ctx.plan_action("armbian-config --cmd CON001 (install Docker Engine)");
    } else {
        plan_apt_install(ctx);
    }
}

fn prefer_armbian_config() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = armbian_present_override() {
            return v;
        }
    }
    which::which("armbian-config").is_ok()
}

fn plan_apt_install(ctx: &mut HostContext) {
    ctx.plan_action("apt update");
    ctx.plan_action(format!("apt install -y {}", APT_PACKAGES.join(" ")));
    ctx.plan_action("systemctl enable --now docker");
}

fn install_docker_engine(ctx: &mut HostContext) -> Result<()> {
    if ctx.is_dry_run() {
        plan_install_actions(ctx);
        return Ok(());
    }
    if prefer_armbian_config() {
        match try_armbian_con001(ctx)? {
            true => return Ok(()),
            false => ctx.log(
                "armbian-config CON001 did not leave docker ready; falling back to apt packages",
            ),
        }
    }
    apt_install_docker(ctx)?;
    enable_docker_service(ctx)?;
    Ok(())
}

fn try_armbian_con001(ctx: &mut HostContext) -> Result<bool> {
    ctx.log("armbian-config --cmd CON001");
    #[cfg(test)]
    {
        if let Some(ok) = armbian_result_override() {
            return Ok(ok && docker_engine_ready());
        }
    }
    let status = Command::new("armbian-config")
        .args(["--cmd", "CON001"])
        .status()
        .map_err(|e| HortoError::command("armbian-config", e.to_string()))?;
    Ok(status.success() && docker_engine_ready())
}

fn apt_install_docker(ctx: &mut HostContext) -> Result<()> {
    #[cfg(test)]
    {
        if skip_real_apt() {
            ctx.log("apt update");
            ctx.log(format!("apt install -y {}", APT_PACKAGES.join(" ")));
            return Ok(());
        }
    }
    apt::apt_update(ctx)?;
    apt::apt_install(ctx, APT_PACKAGES)?;
    Ok(())
}

fn enable_docker_service(ctx: &mut HostContext) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action("systemctl enable --now docker");
        return Ok(());
    }
    ctx.log("systemctl enable --now docker");
    #[cfg(test)]
    {
        if skip_real_systemctl() {
            return Ok(());
        }
    }
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

fn finish_after_install(ctx: &mut HostContext) -> Result<()> {
    if !docker_engine_ready() {
        return Err(HortoError::msg(
            "Docker Engine still missing after install. On Armbian run: sudo armbian-config --cmd CON001",
        ));
    }
    ctx.log("Step d0 complete: Docker Engine ready.");
    Ok(())
}

#[cfg(test)]
thread_local! {
    static READY_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static ARMBIAN_PRESENT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static ARMBIAN_RESULT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static SKIP_APT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SKIP_SYSTEMCTL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn ready_override() -> Option<bool> {
    READY_OVERRIDE.with(std::cell::Cell::get)
}

#[cfg(test)]
fn armbian_present_override() -> Option<bool> {
    ARMBIAN_PRESENT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn armbian_result_override() -> Option<bool> {
    ARMBIAN_RESULT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn skip_real_apt() -> bool {
    SKIP_APT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn skip_real_systemctl() -> bool {
    SKIP_SYSTEMCTL.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_test_hooks() {
    READY_OVERRIDE.with(|c| c.set(None));
    ARMBIAN_PRESENT.with(|c| c.set(None));
    ARMBIAN_RESULT.with(|c| c.set(None));
    SKIP_APT.with(|c| c.set(false));
    SKIP_SYSTEMCTL.with(|c| c.set(false));
}

#[cfg(test)]
pub(crate) fn set_ready_override(value: Option<bool>) {
    READY_OVERRIDE.with(|c| c.set(value));
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
    fn dry_run_when_ready_logs_available() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(true)));
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        D0DockerEngine.apply(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("already available")));
        assert!(D0DockerEngine.is_done(&ctx));
        reset_test_hooks();
    }

    #[test]
    fn dry_run_plans_armbian_when_present() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(true)));
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        D0DockerEngine.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("armbian-config")));
        reset_test_hooks();
    }

    #[test]
    fn dry_run_plans_apt_without_armbian() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(false)));
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let planned = D0DockerEngine.plan(&mut ctx).unwrap();
        assert!(planned.iter().any(|p| p.summary.contains("apt update")));
        assert!(planned.iter().any(|p| p.summary.contains("docker.io")));
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("systemctl enable")));
        reset_test_hooks();
    }

    #[test]
    fn apply_requires_root_when_engine_missing() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        if is_root() {
            reset_test_hooks();
            return;
        }
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        let err = D0DockerEngine.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("requires root"));
        reset_test_hooks();
    }

    #[test]
    fn finish_after_install_ok_and_err() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        let err = finish_after_install(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("still missing"));

        READY_OVERRIDE.with(|c| c.set(Some(true)));
        finish_after_install(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("d0 complete")));
        reset_test_hooks();
    }

    #[test]
    fn install_armbian_success_short_circuits() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(true)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(true)));
        ARMBIAN_RESULT.with(|c| c.set(Some(true)));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        install_docker_engine(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("armbian-config")));
        reset_test_hooks();
    }

    #[test]
    fn install_falls_back_to_apt_after_armbian_miss() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(true)));
        ARMBIAN_RESULT.with(|c| c.set(Some(false)));
        SKIP_APT.with(|c| c.set(true));
        SKIP_SYSTEMCTL.with(|c| c.set(true));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        install_docker_engine(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("falling back to apt")));
        assert!(ctx.logs.iter().any(|l| l.contains("apt install")));
        assert!(ctx.logs.iter().any(|l| l.contains("systemctl enable")));
        reset_test_hooks();
    }

    #[test]
    fn install_apt_path_without_armbian() {
        reset_test_hooks();
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(false)));
        SKIP_APT.with(|c| c.set(true));
        SKIP_SYSTEMCTL.with(|c| c.set(true));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        install_docker_engine(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("apt install")));
        reset_test_hooks();
    }

    #[test]
    fn enable_docker_service_dry_run_plans() {
        reset_test_hooks();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        enable_docker_service(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("systemctl enable")));
    }

    #[test]
    fn apply_full_path_with_hooks_when_root() {
        reset_test_hooks();
        if !is_root() {
            return;
        }
        READY_OVERRIDE.with(|c| c.set(Some(false)));
        ARMBIAN_PRESENT.with(|c| c.set(Some(false)));
        SKIP_APT.with(|c| c.set(true));
        SKIP_SYSTEMCTL.with(|c| c.set(true));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        // Still missing after install helpers.
        let err = D0DockerEngine.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("still missing"));
        reset_test_hooks();
    }
}
