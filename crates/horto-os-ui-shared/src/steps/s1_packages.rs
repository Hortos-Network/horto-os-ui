//! reference: horto-os/scripts/s1_init_horto_os.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::Result;
use crate::kits::apt;
use crate::step::Step;

pub struct S1Packages;

pub const S1_PACKAGES: &[&str] = &[
    "cockpit",
    "cockpit-networkmanager",
    "hostapd",
    "dnsmasq",
    "iptables",
    "avahi-daemon",
];

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
        1
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
        apt::apt_install(ctx, &["cockpit", "cockpit-networkmanager"])?;
        ctx.log("Installing IoT LAN components...");
        apt::apt_install(ctx, &["hostapd", "dnsmasq", "iptables", "avahi-daemon"])?;
        ctx.log("Step s1 complete: base packages installed.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_list_matches_script() {
        assert!(S1_PACKAGES.contains(&"cockpit"));
        assert!(S1_PACKAGES.contains(&"cockpit-networkmanager"));
        assert!(S1_PACKAGES.contains(&"hostapd"));
        assert!(S1_PACKAGES.contains(&"dnsmasq"));
        assert!(S1_PACKAGES.contains(&"iptables"));
        assert!(S1_PACKAGES.contains(&"avahi-daemon"));
        assert_eq!(S1_PACKAGES.len(), 6);
    }
}
