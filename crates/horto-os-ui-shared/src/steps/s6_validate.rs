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

        // `netplan generate` always reads the host /etc/netplan, not ctx.paths.etc.
        // Skip when testing against a temp tree, or when not root (CI / dry hosts).
        if which::which("netplan").is_ok() {
            if etc != std::path::Path::new("/etc") {
                ctx.log("WARNING: etc is not /etc; skipping netplan generate");
            } else if !crate::context::is_root() {
                ctx.log("WARNING: not root; skipping netplan generate");
            } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ApplyMode, NonInteractivePrompts};
    use crate::kits::envfile;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
    use std::collections::BTreeMap;
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

    fn seed_full_env(paths: &HostPaths) {
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("MY_HOSTNAME".into(), "cov-box".into());
        envfile::write(&paths.full_env_file(), &map).unwrap();
    }

    #[test]
    fn trait_metadata_is_stable() {
        let step = S6Validate;
        assert_eq!(step.id(), "s6");
        assert_eq!(step.reference_script(), "s6_validate_configs.sh");
        assert_eq!(step.step_version(), 1);
        assert_eq!(step.depends_on(), &["s5"]);
        assert!(!step.title().is_empty());
        assert!(!step.needs_reboot_after());
        assert!(!step.destructive());
    }

    #[test]
    fn is_done_requires_hostapd_and_netplan() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(paths.etc.join("hostapd")).unwrap();
        std::fs::create_dir_all(paths.etc.join("netplan")).unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        assert!(!S6Validate.is_done(&ctx));

        std::fs::write(
            ctx.paths.etc.join("hostapd/hostapd.conf"),
            b"interface=wlan0\n",
        )
        .unwrap();
        std::fs::write(
            ctx.paths.etc.join("netplan/99-iot-lan.yaml"),
            b"network: {version: 2}\n",
        )
        .unwrap();
        assert!(S6Validate.is_done(&ctx));
    }

    #[test]
    fn plan_records_validate_and_hostapd_actions() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let planned = S6Validate.plan(&mut ctx).unwrap();
        assert!(planned.iter().any(|p| p.summary.contains("validate /etc")));
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("netplan generate")));
        assert!(planned.iter().any(|p| p.summary.contains("hostapd")));
    }

    #[test]
    fn apply_dry_run_without_full_env_calls_plan() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        S6Validate.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("validate /etc")));
    }

    #[test]
    fn apply_dry_run_with_full_env_calls_plan() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        seed_full_env(&paths);
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        S6Validate.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("validate /etc")));
    }

    #[test]
    fn apply_mode_skips_when_full_env_missing() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        S6Validate.apply(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("skipping")));
    }

    #[test]
    fn apply_mode_fails_when_expected_files_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        seed_full_env(&paths);
        std::fs::create_dir_all(&paths.etc).unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_paths(paths)
            .with_prompts(Box::new(NonInteractivePrompts));
        let err = S6Validate.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("validation"));
        assert!(ctx.logs.iter().any(|l| l.contains("ERROR: missing")));
    }

    #[test]
    fn apply_mode_fails_when_placeholder_remains() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        seed_full_env(&paths);
        for dir in ["netplan", "hostapd", "avahi", "dnsmasq.d", "sysctl.d"] {
            std::fs::create_dir_all(paths.etc.join(dir)).unwrap();
        }
        // hosts still has an unreplaced placeholder => check_no_ph returns false.
        std::fs::write(paths.etc.join("hosts"), b"127.0.0.1 {{MY_HOSTNAME}}\n").unwrap();
        std::fs::write(
            paths.etc.join("netplan/99-iot-lan.yaml"),
            b"network: {version: 2}\n",
        )
        .unwrap();
        std::fs::write(paths.etc.join("hostapd/hostapd.conf"), b"interface=wlan0\n").unwrap();
        std::fs::write(paths.etc.join("avahi/avahi-daemon.conf"), b"[server]\n").unwrap();
        std::fs::write(paths.etc.join("avahi/hosts"), b"\n").unwrap();
        std::fs::write(paths.etc.join("resolv.conf"), b"nameserver 1.1.1.1\n").unwrap();
        std::fs::write(paths.etc.join("dnsmasq.d/iot-lan.conf"), b"# ok\n").unwrap();
        std::fs::write(
            paths.etc.join("sysctl.d/packet_forwarding.conf"),
            b"net.ipv4.ip_forward=1\n",
        )
        .unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let err = S6Validate.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("validation"));
        assert!(ctx.logs.iter().any(|l| l.contains("placeholder")));
    }

    #[test]
    fn check_no_ph_reports_missing_and_placeholders() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        assert!(!check_no_ph(&mut ctx, &tmp.path().join("missing")));

        let good = tmp.path().join("good.txt");
        std::fs::write(&good, b"nameserver 1.1.1.1\n").unwrap();
        assert!(check_no_ph(&mut ctx, &good));

        let bad = tmp.path().join("bad.txt");
        std::fs::write(&bad, b"{{PLACEHOLDER}} still here\n").unwrap();
        assert!(!check_no_ph(&mut ctx, &bad));
    }
}
