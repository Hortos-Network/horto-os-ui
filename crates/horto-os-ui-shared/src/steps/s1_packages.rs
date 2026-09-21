//! reference: horto-os/scripts/s1_init_horto_os.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::Result;
use crate::kits::apt;
use crate::step::Step;

/// Install base cockpit packages (`s1`).
pub struct S1Packages;

/// Base packages from tip `s1_init_horto_os.sh` (`IoT` packages move to s2 when `IOT_LAN=y`).
pub const S1_PACKAGES: &[&str] = &["cockpit", "cockpit-networkmanager"];

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
        2
    }
    fn is_done(&self, _ctx: &HostContext) -> bool {
        S1_PACKAGES.iter().all(|p| apt::package_installed(p))
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action("apt update");
        ctx.plan_action(format!("apt install -y {}", S1_PACKAGES.join(" ")));
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        apt::apt_update(ctx)?;
        apt::apt_install(ctx, S1_PACKAGES)?;
        ctx.log("Step s1 complete: base packages installed.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_list_matches_script() {
        assert_eq!(S1_PACKAGES, &["cockpit", "cockpit-networkmanager"]);
    }
}
