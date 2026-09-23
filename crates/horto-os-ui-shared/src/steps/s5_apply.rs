//! Apply staged configs from `active_setup/etc` onto host `/etc` (`s5`).
//!
//! Full IoT-LAN: tip `scripts/networking/s5_apply_configs.sh`.
//! Host-only (no `IoT` env): tip `scripts/s5_apply_host_configs.sh` (`hosts` + `hostname` only).
use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::{fs, systemd};
use crate::step::Step;
use walkdir::WalkDir;

/// Copy staged files into [`HostContext`] `/etc` and disable `systemd-resolved` for `IoT`.
pub struct S5Apply;

impl Step for S5Apply {
    fn id(&self) -> &'static str {
        "s5"
    }
    fn title(&self) -> &'static str {
        "Apply staged configs to /etc"
    }
    fn reference_script(&self) -> &'static str {
        "s5_apply_configs.sh"
    }
    fn step_version(&self) -> u32 {
        2
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s4"]
    }
    fn needs_reboot_after(&self) -> bool {
        true
    }
    fn destructive(&self) -> bool {
        true
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        // Heuristic: hostname file exists under /etc and staging exists.
        ctx.paths.etc.join("hostname").exists() && ctx.paths.staging_etc().is_dir()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        if iot_lan_apply(ctx) {
            ctx.plan_action(format!(
                "copy {}/* -> {}/",
                ctx.paths.staging_etc().display(),
                ctx.paths.etc.display()
            ));
            ctx.plan_action("handle resolv.conf symlink specially");
            ctx.plan_action("systemctl stop/disable systemd-resolved");
        } else {
            ctx.plan_action("apply staged hosts + hostname only (host setup, no IoT-LAN)");
        }
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        let staging = ctx.paths.staging_etc();
        if !staging.is_dir() && !ctx.is_dry_run() {
            return Err(HortoError::msg(format!(
                "staging directory not found: {}",
                staging.display()
            )));
        }
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        if iot_lan_apply(ctx) {
            apply_full_iot(ctx, &staging)?;
        } else {
            apply_host_only(ctx, &staging)?;
        }
        ctx.log("Step s5 complete: staged configuration applied. Reboot recommended for hostname.");
        Ok(())
    }
}

fn iot_lan_apply(ctx: &HostContext) -> bool {
    ctx.paths.full_env_file().exists()
}

fn apply_full_iot(ctx: &mut HostContext, staging: &std::path::Path) -> Result<()> {
    for entry in WalkDir::new(staging)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let staged = entry.path();
        let rel = staged.strip_prefix(staging).unwrap_or(staged).to_path_buf();
        let target = ctx.paths.etc.join(&rel);
        if rel.as_os_str() == "resolv.conf" && (target.is_symlink() || target.exists()) {
            ctx.log("Removing existing resolv.conf before apply...");
            fs::remove_path(ctx, &target)?;
        }
        fs::copy_file(ctx, staged, &target)?;
        let mode = file_mode_for_staged(&rel);
        fs::chmod(ctx, &target, mode)?;
        ctx.log(format!(
            "Applied file: {} -> {}",
            staged.display(),
            target.display()
        ));
    }
    if crate::context::is_root() {
        systemd::try_stop(ctx, "systemd-resolved");
        systemd::try_disable(ctx, "systemd-resolved");
    } else {
        ctx.log("Not root; skipping systemd-resolved stop/disable");
    }
    Ok(())
}

fn apply_host_only(ctx: &mut HostContext, staging: &std::path::Path) -> Result<()> {
    for rel in ["hosts", "hostname"] {
        let staged = staging.join(rel);
        if !staged.is_file() {
            return Err(HortoError::msg(format!(
                "staged {rel} file not found: {}",
                staged.display()
            )));
        }
        let target = ctx.paths.etc.join(rel);
        fs::copy_file(ctx, &staged, &target)?;
        fs::chmod(ctx, &target, 0o644)?;
        ctx.log(format!(
            "Applied: {} -> {}",
            staged.display(),
            target.display()
        ));
    }
    Ok(())
}

fn file_mode_for_staged(rel: &std::path::Path) -> u32 {
    if rel.starts_with("netplan") {
        0o640
    } else {
        0o644
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApplyMode;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
    use std::path::Path;
    use tempfile::TempDir;

    fn temp_paths(root: &std::path::Path) -> HostPaths {
        HostPaths {
            active_setup: root.join("active_setup"),
            backup: root.join("backup"),
            docker: root.join("docker"),
            etc: root.join("etc"),
            lease_file: root.join("leases"),
        }
    }

    #[test]
    fn netplan_files_get_mode_640() {
        assert_eq!(
            file_mode_for_staged(Path::new("netplan/99-iot.yaml")),
            0o640
        );
        assert_eq!(file_mode_for_staged(Path::new("hostname")), 0o644);
        assert_eq!(file_mode_for_staged(Path::new("dnsmasq.d/iot.conf")), 0o644);
    }

    #[test]
    fn apply_host_only_copies_hosts_and_hostname() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let staging = paths.staging_etc();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&paths.etc).unwrap();
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(staging.join("hosts"), b"127.0.0.1 localhost\n").unwrap();
        std::fs::write(staging.join("hostname"), b"horto-box\n").unwrap();
        // No iot-lan_conf.env => host-only path.
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        S5Apply.apply(&mut ctx).unwrap();
        assert_eq!(
            std::fs::read_to_string(ctx.paths.etc.join("hostname")).unwrap(),
            "horto-box\n"
        );
        assert!(ctx
            .logs
            .iter()
            .any(|l| l.contains("Applied:") && l.contains("hosts")));
        assert!(!ctx.logs.iter().any(|l| l.contains("systemd-resolved")));
    }

    #[test]
    fn plan_host_only_when_iot_env_missing() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let planned = S5Apply.plan(&mut ctx).unwrap();
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("hosts + hostname only")));
    }
}
