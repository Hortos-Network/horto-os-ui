use crate::context::HostContext;
use crate::error::Result;

/// One setup / ops step. Implemented by isolated modules under `steps/`.
///
/// Steps are versioned (`step_version`) and may declare dependencies (`depends_on`).
/// Surfaces call [`crate::setup_run`] / [`crate::setup_step`] rather than steps directly.
pub trait Step: Send + Sync {
    /// Stable short id (`s1`, `m1`, `d1`, …).
    fn id(&self) -> &'static str;
    /// Human-readable title for TUI / status.
    fn title(&self) -> &'static str;
    /// Legacy shell script this step absorbs (documentation / tip-sync).
    fn reference_script(&self) -> &'static str;
    /// Schema of the step trait itself (rarely bumped).
    fn schema_version(&self) -> u32 {
        1
    }
    /// Implementation version; resume marks stale when this increases.
    fn step_version(&self) -> u32;
    /// Step ids that must be `Done` before this step may run.
    fn depends_on(&self) -> &'static [&'static str] {
        &[]
    }
    /// Heuristic: already completed on this host layout.
    fn is_done(&self, ctx: &HostContext) -> bool;
    /// Record planned actions without mutating the host (also used in dry-run apply).
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<crate::context::PlannedAction>>;
    /// Execute or plan according to [`HostContext::mode`](crate::ApplyMode).
    fn apply(&self, ctx: &mut HostContext) -> Result<()>;
    /// Advise a reboot after a successful apply.
    fn needs_reboot_after(&self) -> bool {
        false
    }
    /// True when apply can overwrite live host config.
    fn destructive(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ApplyMode, HostContext};
    use crate::pipeline::SetupKind;

    /// Minimal `Step` that leaves trait defaults in place.
    struct DefaultsOnly;

    impl Step for DefaultsOnly {
        fn id(&self) -> &'static str {
            "t0"
        }
        fn title(&self) -> &'static str {
            "defaults-only"
        }
        fn reference_script(&self) -> &'static str {
            "none"
        }
        fn step_version(&self) -> u32 {
            1
        }
        fn is_done(&self, _ctx: &HostContext) -> bool {
            false
        }
        fn plan(&self, _ctx: &mut HostContext) -> Result<Vec<crate::context::PlannedAction>> {
            Ok(Vec::new())
        }
        fn apply(&self, _ctx: &mut HostContext) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn trait_defaults_are_stable() {
        let concrete = DefaultsOnly;
        let step: &dyn Step = &concrete;
        assert_eq!(step.schema_version(), 1);
        assert!(step.depends_on().is_empty());
        assert!(!step.needs_reboot_after());
        assert!(!step.destructive());
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal);
        assert!(!step.is_done(&ctx));
        assert!(step.plan(&mut ctx).unwrap().is_empty());
        step.apply(&mut ctx).unwrap();
        assert_eq!(step.id(), "t0");
        assert_eq!(step.title(), "defaults-only");
        assert_eq!(step.reference_script(), "none");
        assert_eq!(step.step_version(), 1);
    }
}
