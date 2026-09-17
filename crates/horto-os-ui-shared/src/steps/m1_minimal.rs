//! Minimal pipeline step: cockpit packages + hostname env (`m1`).
//!
//! reference: horto-os/scripts/m1_minimal_setup_run.sh
use crate::context::{HostContext, PlannedAction};
use crate::embed;
use crate::error::{HortoError, Result};
use crate::kits::{apt, envfile, fs};
use crate::step::Step;
use std::collections::BTreeMap;

/// Minimal setup: install cockpit (when root) and write `minimal_setup_vars.env`.
pub struct M1Minimal;

impl Step for M1Minimal {
    fn id(&self) -> &'static str {
        "m1"
    }
    fn title(&self) -> &'static str {
        "Minimal: cockpit + hostname env"
    }
    fn reference_script(&self) -> &'static str {
        "m1_minimal_setup_run.sh"
    }
    fn step_version(&self) -> u32 {
        1
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        let path = ctx.paths.minimal_env_file();
        path.exists()
            && envfile::load(&path)
                .ok()
                .and_then(|m| envfile::require_keys(&m, &["MY_HOSTNAME"]).ok())
                .is_some()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action("apt update");
        ctx.plan_action("apt install -y cockpit cockpit-networkmanager");
        ctx.plan_action(format!(
            "write {} with MY_HOSTNAME",
            ctx.paths.minimal_env_file().display()
        ));
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        fs::ensure_dir(ctx, &ctx.paths.active_setup.clone())?;
        if ctx.is_dry_run() || crate::context::is_root() {
            apt::apt_update(ctx)?;
            apt::apt_install(ctx, &["cockpit", "cockpit-networkmanager"])?;
        } else {
            ctx.log("Not root; skipping cockpit apt install");
        }

        let active = ctx.paths.minimal_env_file();
        if ctx.is_dry_run() {
            ctx.plan_action(format!("write {}", active.display()));
            return Ok(());
        }

        let map = if active.exists() {
            envfile::load(&active)?
        } else {
            let template = embed::get_str("config/minimal_setup_vars.env")
                .ok_or_else(|| HortoError::EmbedMissing("config/minimal_setup_vars.env".into()))?;
            envfile::parse(&template)
        };

        let hostname = ctx.prompt(
            "Device hostname",
            map.get("MY_HOSTNAME")
                .map(String::as_str)
                .unwrap_or("Horto-OS_xxx"),
        );
        if hostname.is_empty() {
            return Err(HortoError::msg("MY_HOSTNAME is empty"));
        }
        let mut out = BTreeMap::new();
        envfile::set_key(&mut out, "MY_HOSTNAME", hostname);
        envfile::write(&active, &out)?;
        ctx.log(format!(
            "Loaded minimal deployment variables from {}",
            active.display()
        ));
        ctx.log("Manual network configuration remains your responsibility in minimal mode.");
        Ok(())
    }
}
