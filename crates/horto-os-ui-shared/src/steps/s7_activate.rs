//! reference: horto-os/scripts/s7_activate_services.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::{envfile, fs, systemd};
use crate::ops::leases;
use crate::step::Step;
use std::process::Command;

/// Activate network services after staged configs are validated (`s7`).
pub struct S7Activate;

impl Step for S7Activate {
    fn id(&self) -> &'static str {
        "s7"
    }
    fn title(&self) -> &'static str {
        "Activate network services"
    }
    fn reference_script(&self) -> &'static str {
        "s7_activate_services.sh"
    }
    fn step_version(&self) -> u32 {
        // v3: skip hostapd restart when WIFI_INTERFACE=none
        3
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s6"]
    }
    fn is_done(&self, _ctx: &HostContext) -> bool {
        false
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action("sysctl --system");
        ctx.plan_action("netplan generate && netplan apply");
        ctx.plan_action("restart dnsmasq, avahi-daemon (hostapd when WiFi AP enabled)");
        ctx.plan_action("optional NAT (HORTO_APPLY_NAT=1 or confirm)");
        ctx.plan_action(
            "export DHCP leases + install /etc/cron.d/export_dhcp_leases (horto net export-leases)",
        );
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if !ctx.paths.full_env_file().exists() {
            if ctx.is_dry_run() {
                self.plan(ctx)?;
                return Ok(());
            }
            ctx.log("s7 only needed for IoT-LAN (full) setup; skipping");
            return Ok(());
        }
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }

        if !crate::context::is_root() {
            ctx.log(
                "Not root; skipping privileged activate (sysctl/netplan/services/NAT/cron). Exporting leases only.",
            );
            let assets = ctx.paths.docker_assets();
            fs::ensure_dir(ctx, &assets)?;
            leases::export_dhcp_leases(ctx)?;
            return Ok(());
        }

        ctx.log("Reloading sysctl settings...");
        let _ = Command::new("sysctl").arg("--system").status();

        if which::which("netplan").is_ok() {
            ctx.log("Applying netplan configuration...");
            let _ = Command::new("netplan").arg("generate").status();
            let status = Command::new("netplan").arg("apply").status();
            if let Ok(s) = status {
                if !s.success() {
                    ctx.log("warning: netplan apply returned non-zero");
                }
            }
        } else {
            ctx.log("Skipping netplan apply: netplan command not found.");
        }

        restart_iot_services(ctx);

        let do_nat =
            ctx.apply_nat || ctx.confirm("Apply NAT / masquerade iptables rules now?", false);
        if do_nat {
            apply_nat_rules(ctx)?;
        } else {
            ctx.log("Skipping NAT / masquerade rule setup.");
        }

        ctx.log("Setting up periodic DHCP lease export...");
        let assets = ctx.paths.docker_assets();
        fs::ensure_dir(ctx, &assets)?;
        leases::export_dhcp_leases(ctx)?;

        install_export_cron(ctx)?;
        ctx.log("Step s7 complete: applied configuration activated.");
        Ok(())
    }
}

fn restart_iot_services(ctx: &mut HostContext) {
    let wifi =
        envfile::load(&ctx.paths.full_env_file()).is_ok_and(|m| envfile::wifi_ap_enabled(&m));
    restart_if_present(ctx, "dnsmasq");
    if wifi {
        restart_if_present(ctx, "hostapd");
    } else {
        ctx.log("WIFI_INTERFACE=none; skipping hostapd restart");
    }
    restart_if_present(ctx, "avahi-daemon");
}

fn restart_if_present(ctx: &mut HostContext, unit: &str) {
    if systemd::unit_present(unit) || which::which("systemctl").is_ok() {
        ctx.log(format!("Restarting {unit}..."));
        systemd::try_restart(ctx, unit);
        systemd::try_enable(ctx, unit);
    } else {
        ctx.log(format!("Skipping {unit}: service not installed."));
    }
}

fn apply_nat_rules(ctx: &mut HostContext) -> Result<()> {
    let map = envfile::load(&ctx.paths.full_env_file()).unwrap_or_default();
    let wan = map
        .get("ETH_LAN")
        .cloned()
        .or_else(default_wan_iface)
        .unwrap_or_else(|| "eth0".into());
    ctx.log(format!(
        "Applying NAT / masquerade rules for interface '{wan}'..."
    ));
    if ctx.is_dry_run() {
        ctx.plan_action(format!(
            "iptables -t nat -A POSTROUTING -o {wan} -j MASQUERADE"
        ));
        ctx.plan_action(format!("iptables -A FORWARD -i br0 -o {wan} -j ACCEPT"));
        ctx.plan_action(format!(
            "iptables -A FORWARD -i {wan} -o br0 -m state --state RELATED,ESTABLISHED -j ACCEPT"
        ));
        return Ok(());
    }
    ensure_nat_masquerade(ctx, &wan)?;
    ensure_forward_accept(ctx, &wan);
    ctx.log("Applied NAT / masquerade rules.");
    maybe_persist_iptables(ctx)?;
    Ok(())
}

fn ensure_nat_masquerade(ctx: &mut HostContext, wan: &str) -> Result<()> {
    run_iptables(
        ctx,
        &[
            "-t",
            "nat",
            "-C",
            "POSTROUTING",
            "-o",
            wan,
            "-j",
            "MASQUERADE",
        ],
    )
    .or_else(|_| {
        run_iptables(
            ctx,
            &[
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-o",
                wan,
                "-j",
                "MASQUERADE",
            ],
        )
    })
}

fn ensure_forward_accept(ctx: &mut HostContext, wan: &str) {
    let _ = run_iptables(
        ctx,
        &["-C", "FORWARD", "-i", "br0", "-o", wan, "-j", "ACCEPT"],
    )
    .or_else(|_| {
        run_iptables(
            ctx,
            &["-A", "FORWARD", "-i", "br0", "-o", wan, "-j", "ACCEPT"],
        )
    });
    let _ = run_iptables(
        ctx,
        &[
            "-C",
            "FORWARD",
            "-i",
            wan,
            "-o",
            "br0",
            "-m",
            "state",
            "--state",
            "RELATED,ESTABLISHED",
            "-j",
            "ACCEPT",
        ],
    )
    .or_else(|_| {
        run_iptables(
            ctx,
            &[
                "-A",
                "FORWARD",
                "-i",
                wan,
                "-o",
                "br0",
                "-m",
                "state",
                "--state",
                "RELATED,ESTABLISHED",
                "-j",
                "ACCEPT",
            ],
        )
    });
}

fn maybe_persist_iptables(ctx: &mut HostContext) -> Result<()> {
    if ctx.confirm(
        "Install iptables-persistent to save these rules across reboot?",
        false,
    ) {
        crate::kits::apt::apt_install(ctx, &["iptables-persistent"])?;
        if crate::context::is_root() {
            let _ = Command::new("netfilter-persistent").arg("save").status();
        }
        ctx.log("Saved iptables rules via iptables-persistent.");
    }
    Ok(())
}

/// Parse `ip route show default` output for the WAN device name.
#[must_use]
pub(crate) fn parse_default_wan_iface(text: &str) -> Option<String> {
    let mut prev = "";
    for part in text.split_whitespace() {
        if prev == "dev" {
            return Some(part.to_string());
        }
        prev = part;
    }
    None
}

fn default_wan_iface() -> Option<String> {
    let output = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_default_wan_iface(&text)
}

fn run_iptables(ctx: &mut HostContext, args: &[&str]) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("iptables {}", args.join(" ")));
        return Ok(());
    }
    let status = Command::new("iptables")
        .args(args)
        .status()
        .map_err(|e| HortoError::msg(format!("iptables: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(HortoError::msg(format!(
            "iptables {} failed",
            args.join(" ")
        )))
    }
}

fn install_export_cron(ctx: &mut HostContext) -> Result<()> {
    // Align with horto-os s7: system cron.d entry as root, not a user crontab.
    // Call horto (resolved absolute when possible) so the box need not keep horto-os scripts.
    let horto_bin =
        which::which("horto").map_or_else(|_| "horto".into(), |p| p.display().to_string());
    let cron_dir = ctx.paths.etc.join("cron.d");
    let cron_file = cron_dir.join("export_dhcp_leases");
    let cron_body = format!("* * * * * root {horto_bin} net export-leases >/dev/null 2>&1\n");

    if ctx.is_dry_run() {
        ctx.plan_action("ensure package cron is installed");
        ctx.plan_action(format!(
            "write {} with: {}",
            cron_file.display(),
            cron_body.trim()
        ));
        ctx.plan_action("systemctl enable --now cron");
        return Ok(());
    }

    if !crate::kits::apt::package_installed("cron") {
        if crate::context::is_root() {
            ctx.log("Installing cron service...");
            crate::kits::apt::apt_update(ctx)?;
            crate::kits::apt::apt_install(ctx, &["cron"])?;
        } else {
            ctx.log("cron package missing and not root; writing cron.d entry only");
        }
    }

    fs::ensure_dir(ctx, &cron_dir)?;
    fs::write_file(ctx, &cron_file, cron_body.as_bytes())?;
    fs::chmod(ctx, &cron_file, 0o644)?;
    if crate::context::is_root() {
        systemd::try_enable(ctx, "cron");
        let _ = Command::new("systemctl").args(["start", "cron"]).status();
    }
    ctx.log(format!(
        "cron job installed in {} (every minute: {horto_bin} net export-leases)",
        cron_file.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ApplyMode, NonInteractivePrompts};
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
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
    fn parse_wan_from_ip_route() {
        assert_eq!(
            parse_default_wan_iface("default via 1.1.1.1 dev eth0 proto dhcp"),
            Some("eth0".into())
        );
        assert!(parse_default_wan_iface("unreachable default").is_none());
    }

    #[test]
    fn nat_rules_dry_run_plans_iptables() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = std::collections::BTreeMap::new();
        map.insert("ETH_LAN".into(), "enp1s0".into());
        crate::kits::envfile::write(&paths.full_env_file(), &map).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        apply_nat_rules(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("MASQUERADE") && p.summary.contains("enp1s0")));
    }

    #[test]
    fn install_cron_writes_under_etc() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.etc).unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_paths(paths)
            .with_prompts(Box::new(NonInteractivePrompts));
        install_export_cron(&mut ctx).unwrap();
        let cron = ctx.paths.etc.join("cron.d/export_dhcp_leases");
        assert!(cron.is_file());
        let body = std::fs::read_to_string(&cron).unwrap();
        assert!(body.contains("export-leases"));
    }

    #[test]
    fn restart_if_present_logs() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        restart_if_present(&mut ctx, "dnsmasq");
        assert!(!ctx.logs.is_empty() || !ctx.planned.is_empty());
    }

    #[test]
    fn restart_iot_services_skips_hostapd_when_wifi_none() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = std::collections::BTreeMap::new();
        map.insert("WIFI_INTERFACE".into(), "none".into());
        crate::kits::envfile::write(&paths.full_env_file(), &map).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        restart_iot_services(&mut ctx);
        assert!(ctx
            .logs
            .iter()
            .any(|l| l.contains("skipping hostapd restart")));
    }

    #[test]
    fn restart_iot_services_skips_hostapd_when_env_missing() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        restart_iot_services(&mut ctx);
        assert!(ctx
            .logs
            .iter()
            .any(|l| l.contains("skipping hostapd restart")));
    }

    #[test]
    fn restart_iot_services_restarts_hostapd_when_wifi_set() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = std::collections::BTreeMap::new();
        map.insert("WIFI_INTERFACE".into(), "wlan0".into());
        crate::kits::envfile::write(&paths.full_env_file(), &map).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        restart_iot_services(&mut ctx);
        assert!(ctx.logs.iter().any(|l| l.contains("Restarting hostapd")));
        assert!(!ctx.logs.iter().any(|l| l.contains("skipping hostapd")));
    }

    #[test]
    fn trait_metadata_is_stable() {
        let step = S7Activate;
        assert_eq!(step.id(), "s7");
        assert_eq!(step.reference_script(), "s7_activate_services.sh");
        assert_eq!(step.step_version(), 3);
        assert_eq!(step.depends_on(), &["s6"]);
        assert_ne!(step.title(), "");
        assert!(!step.is_done(&HostContext::new(ApplyMode::DryRun, SetupKind::Full)));
    }

    #[test]
    fn plan_records_activation_actions() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let planned = S7Activate.plan(&mut ctx).unwrap();
        assert!(planned.iter().any(|p| p.summary.contains("sysctl")));
        assert!(planned.iter().any(|p| p.summary.contains("netplan")));
        assert!(planned.iter().any(|p| p.summary.contains("dnsmasq")));
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("hostapd when WiFi AP enabled")));
        assert!(planned.iter().any(|p| p.summary.contains("DHCP leases")));
    }

    #[test]
    fn apply_dry_run_without_full_env_still_plans() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        S7Activate.apply(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("sysctl")));
    }

    #[test]
    fn apply_dry_run_with_full_env_plans_activation() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = std::collections::BTreeMap::new();
        map.insert("MY_HOSTNAME".into(), "cov-box".into());
        crate::kits::envfile::write(&paths.full_env_file(), &map).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        S7Activate.apply(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("netplan")));
    }

    #[test]
    fn install_cron_dry_run_records_plan_actions() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        install_export_cron(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("cron.d")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("systemctl enable")));
    }

    #[test]
    fn nat_rules_dry_run_uses_default_iface_when_missing_env_key() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        // Write env without ETH_LAN so default_wan_iface / fallback "eth0" is used.
        let map = std::collections::BTreeMap::new();
        crate::kits::envfile::write(&paths.full_env_file(), &map).unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        apply_nat_rules(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("MASQUERADE")));
        assert!(ctx.planned.iter().any(|p| p.summary.contains("FORWARD")));
    }

    #[test]
    fn run_iptables_dry_run_plans_only() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        run_iptables(&mut ctx, &["-A", "FORWARD", "-i", "br0"]).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.starts_with("iptables") && p.summary.contains("FORWARD")));
    }

    #[test]
    fn parse_default_wan_iface_handles_multiple_hops() {
        assert_eq!(
            parse_default_wan_iface("default via 10.0.0.1 dev enp2s0 proto static"),
            Some("enp2s0".into())
        );
        assert_eq!(parse_default_wan_iface(""), None);
        assert_eq!(parse_default_wan_iface("dev"), None);
    }

    #[test]
    fn ensure_nat_helpers_dry_run_plan_iptables() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        ensure_nat_masquerade(&mut ctx, "enp1s0").unwrap();
        ensure_forward_accept(&mut ctx, "enp1s0");
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("MASQUERADE") && p.summary.contains("enp1s0")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("FORWARD") && p.summary.contains("br0")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("RELATED,ESTABLISHED")));
    }

    #[test]
    fn maybe_persist_iptables_skips_when_confirm_false() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full)
            .with_paths(temp_paths(tmp.path()))
            .with_prompts(Box::new(NonInteractivePrompts));
        maybe_persist_iptables(&mut ctx).unwrap();
        assert!(!ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("iptables-persistent")));
        assert!(!ctx.logs.iter().any(|l| l.contains("iptables-persistent")));
    }

    #[test]
    fn maybe_persist_iptables_plans_apt_when_confirm_true() {
        struct YesPrompts;
        impl crate::context::PromptsProvider for YesPrompts {
            fn prompt(&mut self, _label: &str, default: &str) -> String {
                default.to_owned()
            }
            fn confirm(&mut self, _question: &str, _default_yes: bool) -> bool {
                true
            }
        }

        let tmp = TempDir::new().unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full)
            .with_paths(temp_paths(tmp.path()))
            .with_prompts(Box::new(YesPrompts));
        maybe_persist_iptables(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("iptables-persistent")));
        assert!(ctx.logs.iter().any(|l| l.contains("iptables-persistent")));
    }

    #[test]
    fn apply_without_full_env_skips_when_not_dry_run() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_paths(temp_paths(tmp.path()))
            .with_prompts(Box::new(NonInteractivePrompts));
        S7Activate.apply(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("skipping")));
        assert_eq!(ctx.planned.len(), 0);
    }
}
