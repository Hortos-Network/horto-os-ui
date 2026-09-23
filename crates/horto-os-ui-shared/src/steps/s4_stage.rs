//! reference: horto-os/scripts/s4_deploy_configs.sh
use crate::context::{HostContext, PlannedAction};
use crate::embed;
use crate::error::{HortoError, Result};
use crate::kits::{envfile, fs, template};
use crate::step::Step;
use std::collections::BTreeMap;

/// Render and stage configs under `active_setup/etc` (`s4`).
pub struct S4Stage;

impl Step for S4Stage {
    fn id(&self) -> &'static str {
        "s4"
    }
    fn title(&self) -> &'static str {
        "Stage configs into active_setup/etc"
    }
    fn reference_script(&self) -> &'static str {
        "s4_deploy_configs.sh"
    }
    fn step_version(&self) -> u32 {
        3
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s3"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        let staging = ctx.paths.staging_etc();
        staging.join("hostname").exists() && staging.join("hosts").exists()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let mode = detect_mode(ctx);
        ctx.plan_action(format!(
            "stage embedded config/ into {} ({mode} mode)",
            ctx.paths.staging_etc().display()
        ));
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        let staging = ctx.paths.staging_etc();
        fs::ensure_dir(ctx, &staging)?;
        let (mode, vars) = load_vars(ctx)?;
        match mode.as_str() {
            "full" => {
                envfile::require_keys(&vars, &["MY_HOSTNAME", "ETH_LAN", "ETH_IOT1"])?;
                render_stage(ctx, "hosts", &vars)?;
                render_stage(ctx, "hostname", &vars)?;
                if envfile::wifi_ap_enabled(&vars) {
                    envfile::require_keys(&vars, &["WIFI_INTERFACE", "WIFI_SSID"])?;
                    render_stage(ctx, "hostapd/hostapd.conf", &vars)?;
                } else {
                    ctx.log("WIFI_INTERFACE=none; skipping hostapd staging");
                }
                render_stage(ctx, "netplan/99-iot-lan.yaml", &vars)?;
                stage_static(ctx, "resolv.conf")?;
                stage_static(ctx, "dnsmasq.d/iot-lan.conf")?;
                stage_static(ctx, "sysctl.d/packet_forwarding.conf")?;
                stage_static(ctx, "avahi/avahi-daemon.conf")?;
                stage_static(ctx, "avahi/hosts")?;
            }
            "minimal" | "os" => {
                envfile::require_keys(&vars, &["MY_HOSTNAME"])?;
                render_stage(ctx, "hosts", &vars)?;
                render_stage(ctx, "hostname", &vars)?;
                stage_static(ctx, "resolv.conf")?;
            }
            other => return Err(HortoError::msg(format!("unknown stage mode: {other}"))),
        }
        let netplan = staging.join("netplan/99-iot-lan.yaml");
        if netplan.exists() || ctx.is_dry_run() {
            fs::chmod(ctx, &netplan, 0o640)?;
            let netplan_dir = staging.join("netplan");
            if netplan_dir.is_dir() {
                for entry in std::fs::read_dir(&netplan_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if entry.path().is_file() {
                        fs::chmod(ctx, &entry.path(), 0o640)?;
                    }
                }
            }
        }
        ctx.log(format!(
            "Step s4 complete: staged in {} using {mode} mode",
            staging.display()
        ));
        Ok(())
    }
}

fn detect_mode(ctx: &HostContext) -> String {
    if ctx.paths.full_env_file().exists() {
        return "full".into();
    }
    if ctx.paths.minimal_env_file().exists() {
        return "minimal".into();
    }
    if ctx.paths.os_configuration_file().exists() {
        return "os".into();
    }
    match ctx.setup_kind {
        crate::pipeline::SetupKind::Full => "full".into(),
        crate::pipeline::SetupKind::Minimal => "minimal".into(),
    }
}

fn load_vars(ctx: &HostContext) -> Result<(String, BTreeMap<String, String>)> {
    if ctx.paths.full_env_file().exists() {
        return Ok(("full".into(), envfile::load(&ctx.paths.full_env_file())?));
    }
    if ctx.paths.minimal_env_file().exists() {
        return Ok((
            "minimal".into(),
            envfile::load(&ctx.paths.minimal_env_file())?,
        ));
    }
    if ctx.paths.os_configuration_file().exists() {
        return Ok((
            "os".into(),
            envfile::load(&ctx.paths.os_configuration_file())?,
        ));
    }
    if ctx.is_dry_run() {
        let mut demo = BTreeMap::new();
        demo.insert("MY_HOSTNAME".into(), "horto-dryrun".into());
        demo.insert("ETH_LAN".into(), "wan".into());
        demo.insert("ETH_IOT1".into(), "lan1".into());
        demo.insert("WIFI_INTERFACE".into(), "none".into());
        demo.insert("WIFI_SSID".into(), "Horto-IoT-LAN".into());
        let mode = match ctx.setup_kind {
            crate::pipeline::SetupKind::Full => "full",
            crate::pipeline::SetupKind::Minimal => "minimal",
        };
        return Ok((mode.into(), demo));
    }
    Err(HortoError::msg(
        "no active setup file found; run s2 or m1 first",
    ))
}

fn render_stage(ctx: &mut HostContext, rel: &str, vars: &BTreeMap<String, String>) -> Result<()> {
    let embed_path = format!("config/{rel}");
    let raw = embed::get_str(&embed_path)
        .ok_or_else(|| HortoError::msg(format!("missing embedded {embed_path}")))?;
    let rendered = template::render(&raw, vars);
    let dest = ctx.paths.staging_etc().join(rel);
    fs::write_file(ctx, &dest, rendered.as_bytes())?;
    ctx.log(format!("Rendered and staged: {rel}"));
    Ok(())
}

fn stage_static(ctx: &mut HostContext, rel: &str) -> Result<()> {
    let embed_path = format!("config/{rel}");
    let data = embed::get(&embed_path)
        .ok_or_else(|| HortoError::msg(format!("missing embedded {embed_path}")))?;
    let dest = ctx.paths.staging_etc().join(rel);
    fs::write_file(ctx, &dest, &data)?;
    ctx.log(format!("Staged static file: {rel}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApplyMode;
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
    fn detect_mode_prefers_existing_env_files() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(paths.full_env_file(), b"MY_HOSTNAME=a\n").unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal).with_paths(paths);
        assert_eq!(detect_mode(&ctx), "full");

        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(paths.minimal_env_file(), b"MY_HOSTNAME=b\n").unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        assert_eq!(detect_mode(&ctx), "minimal");

        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(paths.os_configuration_file(), b"OS_TYPE=debian\n").unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        assert_eq!(detect_mode(&ctx), "os");
    }

    #[test]
    fn detect_mode_falls_back_to_setup_kind() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        assert_eq!(detect_mode(&ctx), "full");

        let tmp = TempDir::new().unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal)
            .with_paths(temp_paths(tmp.path()));
        assert_eq!(detect_mode(&ctx), "minimal");
    }

    #[test]
    fn plan_uses_detect_mode_label() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(paths.minimal_env_file(), b"MY_HOSTNAME=stage\n").unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        let planned = S4Stage.plan(&mut ctx).unwrap();
        assert!(planned.iter().any(|p| p.summary.contains("minimal mode")));
    }
}
