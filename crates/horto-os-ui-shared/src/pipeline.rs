//! Ordered Full / Minimal step registries and id lookup.

use crate::step::Step;
use crate::steps::{
    d0_docker_engine::D0DockerEngine, d1_docker::D1Docker, d2_start_stacks::D2StartStacks,
    m1_minimal::M1Minimal, s1_packages::S1Packages, s2_env::S2Env, s3_backup::S3Backup,
    s4_stage::S4Stage, s5_apply::S5Apply, s6_validate::S6Validate, s7_activate::S7Activate,
};
use std::sync::OnceLock;

/// Which ordered pipeline to run or report status for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupKind {
    /// Full IoT-LAN pipeline (`s1`…`s7` + `d0`…`d2`).
    Full,
    /// Minimal pipeline (`m1`, `s3`, `s4`).
    Minimal,
}

impl SetupKind {
    /// Stable string used in status JSON and CLI output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Minimal => "minimal",
        }
    }
}

static FULL: OnceLock<Vec<&'static dyn Step>> = OnceLock::new();
static MINIMAL: OnceLock<Vec<&'static dyn Step>> = OnceLock::new();
static ALL: OnceLock<Vec<&'static dyn Step>> = OnceLock::new();

fn full_steps() -> &'static [&'static dyn Step] {
    FULL.get_or_init(|| {
        vec![
            &S1Packages,
            &S2Env,
            &S3Backup,
            &S4Stage,
            &S5Apply,
            &S6Validate,
            &S7Activate,
            &D0DockerEngine,
            &D1Docker,
            &D2StartStacks,
        ]
    })
}

fn minimal_steps() -> &'static [&'static dyn Step] {
    MINIMAL.get_or_init(|| vec![&M1Minimal, &S3Backup, &S4Stage])
}

fn all_registered() -> &'static [&'static dyn Step] {
    ALL.get_or_init(|| {
        vec![
            &S1Packages,
            &S2Env,
            &S3Backup,
            &S4Stage,
            &S5Apply,
            &S6Validate,
            &S7Activate,
            &M1Minimal,
            &D0DockerEngine,
            &D1Docker,
            &D2StartStacks,
        ]
    })
}

/// Ordered steps for `kind` (Full or Minimal).
#[must_use]
pub fn pipeline(kind: SetupKind) -> &'static [&'static dyn Step] {
    match kind {
        SetupKind::Full => full_steps(),
        SetupKind::Minimal => minimal_steps(),
    }
}

/// Look up a registered step by id (`s1`, `d1`, …).
///
/// Searches the union of Full and Minimal registries (including steps only on Full).
///
/// # Examples
///
/// ```
/// use horto_os_ui_shared::lookup;
///
/// let s1 = lookup("s1").expect("s1 registered");
/// assert_eq!(s1.id(), "s1");
/// assert!(lookup("nope").is_none());
/// ```
#[must_use]
pub fn lookup(id: &str) -> Option<&'static dyn Step> {
    all_registered().iter().copied().find(|s| s.id() == id)
}

/// Step ids in Full pipeline order.
#[must_use]
pub fn full_ids() -> Vec<&'static str> {
    full_steps().iter().map(|s| s.id()).collect()
}

/// Step ids in Minimal pipeline order.
#[must_use]
pub fn minimal_ids() -> Vec<&'static str> {
    minimal_steps().iter().map(|s| s.id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_registry_ids() {
        assert_eq!(
            full_ids(),
            vec!["s1", "s2", "s3", "s4", "s5", "s6", "s7", "d0", "d1", "d2"]
        );
    }

    #[test]
    fn minimal_registry_ids() {
        assert_eq!(minimal_ids(), vec!["m1", "s3", "s4"]);
    }

    #[test]
    fn docker_steps_only_on_full() {
        assert!(full_ids().contains(&"d0"));
        assert!(full_ids().contains(&"d1"));
        assert!(full_ids().contains(&"d2"));
        assert!(!minimal_ids().contains(&"d0"));
        assert!(!minimal_ids().contains(&"d1"));
        assert!(!minimal_ids().contains(&"d2"));
    }
}
