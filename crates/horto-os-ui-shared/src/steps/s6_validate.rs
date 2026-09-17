//! reference: horto-os/scripts/s6_validate_configs.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::{systemd, template};
use crate::step::Step;
use std::fs;
use std::process::Command;

pub struct S6Validate;

impl Step for S6Validate {
    fn id(&self) -> &'static str {
        "s6"
    }
    fn title(&self) -> &'static str {
        "Validate applied configs and enable hostapd"
    }
    fn reference_script(&self) -> &'static str {
        "s6_validate_configs.sh"
    }
    fn step_version(&self) -> u32 {
        1
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s5"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        let etc = &ctx.paths.etc;
        etc.join("hostapd/hostapd.conf").exists() && etc.join("netplan/99-iot-lan.yaml").exists()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action("validate /etc hosts, netplan, hostapd, avahi, resolv, dnsmasq, sysctl");
        ctx.plan_action("netplan generate");
        ctx.plan_action("systemctl unmask/enable/start hostapd");
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if !ctx.paths.full_env_file().exists() {
            if ctx.is_dry_run() {
                self.plan(ctx)?;
                return Ok(());
            }
            ctx.log("s6 only needed for IoT-LAN (full) setup; skipping");
            return Ok(());
        }
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        let etc = ctx.paths.etc.clone();
        let mut failed = false;
        failed |= !check_exists(ctx, &etc.join("hosts"));
        failed |= !check_no_ph(ctx, &etc.join("hosts"));
        failed |= !check_exists(ctx, &etc.join("netplan/99-iot-lan.yaml"));
        failed |= !check_no_ph(ctx, &etc.join("netplan/99-iot-lan.yaml"));
        failed |= !check_exists(ctx, &etc.join("hostapd/hostapd.conf"));
        failed |= !check_no_ph(ctx, &etc.join("hostapd/hostapd.conf"));
        failed |= !check_exists(ctx, &etc.join("avahi/avahi-daemon.conf"));
        failed |= !check_exists(ctx, &etc.join("avahi/hosts"));
        failed |= !check_exists(ctx, &etc.join("resolv.conf"));
        failed |= !check_exists(ctx, &etc.join("dnsmasq.d/iot-lan.conf"));
        failed |= !check_exists(ctx, &etc.join("sysctl.d/packet_forwarding.conf"));

        if which::which("netplan").is_ok() {
            let status = Command::new("netplan").arg("generate").status();
            match status {
                Ok(s) if s.success() => ctx.log("OK: netplan generate succeeded"),
                Ok(_) => {
                    ctx.log("ERROR: netplan generate failed");
                    failed = true;
                }
                Err(e) => {
                    ctx.log(format!("ERROR: netplan generate: {e}"));
                    failed = true;
                }
            }
        } else {
            ctx.log("WARNING: netplan command not found; skipping netplan validation");
        }

        if failed {
            return Err(HortoError::msg(
                "Step s6 failed: configuration validation found errors",
            ));
        }

        ctx.log("Enabling hostapd...");
        if crate::context::is_root() {
            systemd::try_unmask(ctx, "hostapd");
            systemd::try_enable(ctx, "hostapd");
            systemd::try_start(ctx, "hostapd");
        } else {
            ctx.log("Not root; skip hostapd unmask/enable/start (avoids polkit prompts).");
        }
        ctx.log("Step s6 complete: configuration validation passed.");
        Ok(())
    }
}

fn check_exists(ctx: &mut HostContext, path: &std::path::Path) -> bool {
    if path.is_file() {
        ctx.log(format!("OK: file exists: {}", path.display()));
        true
    } else {
        ctx.log(format!("ERROR: missing file: {}", path.display()));
        false
    }
}

fn check_no_ph(ctx: &mut HostContext, path: &std::path::Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        ctx.log(format!(
            "ERROR: cannot validate missing file: {}",
            path.display()
        ));
        return false;
    };
    if template::has_unreplaced_placeholders(&text) {
        ctx.log(format!(
            "ERROR: unreplaced placeholder found in {}",
            path.display()
        ));
        false
    } else {
        ctx.log(format!("OK: no placeholders remain in {}", path.display()));
        true
    }
}
