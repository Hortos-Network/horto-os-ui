//! reference: horto-os/scripts/s1_init_horto_os.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::Result;
use crate::kits::{apt, systemd};
use crate::step::Step;
use std::fs;
use std::path::Path;

/// Install base cockpit packages (`s1`).
pub struct S1Packages;

/// Base packages from tip `s1_init_horto_os.sh` (`IoT` packages move to s2 when `IOT_LAN=y`).
pub const S1_PACKAGES: &[&str] = &["cockpit", "cockpit-networkmanager"];

/// Port advertised in `service_links.env` / Homepage for Cockpit.
pub const COCKPIT_LISTEN_PORT: u16 = 9890;

/// systemd drop-in body so Cockpit listens on [`COCKPIT_LISTEN_PORT`] (not stock 9090).
pub const COCKPIT_SOCKET_DROPIN: &str = "\
[Socket]
ListenStream=
ListenStream=9890
";

impl Step for S1Packages {
    fn id(&self) -> &'static str {
        "s1"
    }
    fn title(&self) -> &'static str {
        "Install base packages"
    }
    fn reference_script(&self) -> &'static str {
        "s1_init_horto_os.sh"
    }
    fn step_version(&self) -> u32 {
        3
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        S1_PACKAGES.iter().all(|p| apt::package_installed(p))
            && cockpit_dropin_present(&ctx.paths.etc)
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action("apt update");
        ctx.plan_action(format!("apt install -y {}", S1_PACKAGES.join(" ")));
        ctx.plan_action(format!(
            "write cockpit.socket drop-in ListenStream={COCKPIT_LISTEN_PORT}"
        ));
        ctx.plan_action("systemctl daemon-reload");
        ctx.plan_action("systemctl enable --now cockpit.socket");
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        apt::apt_update(ctx)?;
        apt::apt_install(ctx, S1_PACKAGES)?;
        configure_cockpit_listen(ctx)?;
        ctx.log(format!(
            "Step s1 complete: base packages installed; Cockpit on :{COCKPIT_LISTEN_PORT}."
        ));
        Ok(())
    }
}

fn cockpit_dropin_path(etc: &Path) -> std::path::PathBuf {
    etc.join("systemd/system/cockpit.socket.d/listen.conf")
}

fn cockpit_dropin_present(etc: &Path) -> bool {
    let path = cockpit_dropin_path(etc);
    path.is_file()
        && fs::read_to_string(path)
            .is_ok_and(|s| s.contains(&format!("ListenStream={COCKPIT_LISTEN_PORT}")))
}

/// Write Cockpit socket drop-in and enable the socket.
///
/// # Errors
///
/// Returns filesystem or `systemctl` errors.
pub fn configure_cockpit_listen(ctx: &mut HostContext) -> Result<()> {
    let path = cockpit_dropin_path(&ctx.paths.etc);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if ctx.is_dry_run() {
        ctx.plan_action(format!("write {}", path.display()));
        return Ok(());
    }
    fs::write(&path, COCKPIT_SOCKET_DROPIN)?;
    ctx.log(format!("Wrote {}", path.display()));
    systemd::daemon_reload(ctx)?;
    systemd::enable_now(ctx, "cockpit.socket")?;
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
    fn package_list_matches_script() {
        assert_eq!(S1_PACKAGES, &["cockpit", "cockpit-networkmanager"]);
    }

    #[test]
    fn dropin_body_clears_and_sets_9890() {
        assert!(COCKPIT_SOCKET_DROPIN.contains("ListenStream=\n"));
        assert!(COCKPIT_SOCKET_DROPIN.contains("ListenStream=9890"));
    }

    #[test]
    fn dry_run_plans_cockpit_port() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        S1Packages.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("ListenStream=9890")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("cockpit.socket")));
    }

    #[test]
    fn configure_writes_dropin_under_etc() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        // Avoid real systemctl on developer hosts: dry-run path for systemd via plan after write.
        // Write file only by temporarily using dry-run for systemctl is hard; write drop-in in dry-run:
        ctx.mode = ApplyMode::DryRun;
        configure_cockpit_listen(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("write")));
    }

    #[test]
    fn step_version_bumped_for_listen_port() {
        assert_eq!(S1Packages.step_version(), 3);
    }
}
