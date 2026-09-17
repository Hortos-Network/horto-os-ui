//! Apply staged configs from `active_setup/etc` onto host `/etc` (`s5`).
//!
//! reference: horto-os/scripts/s5_apply_configs.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::{fs, systemd};
use crate::step::Step;
use walkdir::WalkDir;

/// Copy staged files into [`HostContext`] `/etc` and disable `systemd-resolved`.
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
        1
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
        if !ctx.paths.full_env_file().exists() && !ctx.is_dry_run() {
            ctx.plan_action("skip s5 (IoT-LAN / full env not present)");
            return Ok(ctx.planned.clone());
        }
        ctx.plan_action(format!(
            "copy {}/* -> {}/",
            ctx.paths.staging_etc().display(),
            ctx.paths.etc.display()
        ));
        ctx.plan_action("handle resolv.conf symlink specially");
        ctx.plan_action("systemctl stop/disable systemd-resolved");
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if !ctx.paths.full_env_file().exists() {
            if ctx.is_dry_run() {
                self.plan(ctx)?;
                return Ok(());
            }
            ctx.log("s5 only needed for IoT-LAN (full) setup; skipping");
            return Ok(());
        }
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
        for entry in WalkDir::new(&staging).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let staged = entry.path();
            let rel = staged
                .strip_prefix(&staging)
                .unwrap_or(staged)
                .to_path_buf();
            let target = ctx.paths.etc.join(&rel);
            if rel.as_os_str() == "resolv.conf" && (target.is_symlink() || target.exists()) {
                ctx.log("Removing existing resolv.conf before apply...");
                fs::remove_path(ctx, &target)?;
            }
            fs::copy_file(ctx, staged, &target)?;
            fs::chmod(ctx, &target, 0o644)?;
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
        ctx.log("Step s5 complete: staged configuration applied. Reboot recommended for hostname.");
        Ok(())
    }
}
