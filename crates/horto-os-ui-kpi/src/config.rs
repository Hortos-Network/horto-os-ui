//! Dashboard panel selection (control-room layout).

use std::collections::BTreeSet;

/// Which chart panels to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardConfig {
    pub energy: bool,
    pub fleet: bool,
    pub readiness: bool,
    pub network: bool,
    pub history: usize,
    pub poll_secs: u64,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            energy: true,
            fleet: true,
            readiness: true,
            network: true,
            history: 60,
            poll_secs: 2,
        }
    }
}

impl DashboardConfig {
    /// Parse `energy,fleet,readiness,network` (empty = all defaults).
    #[must_use]
    pub fn from_panels_csv(raw: &str) -> Self {
        let mut cfg = Self::default();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return cfg;
        }
        let set: BTreeSet<_> = trimmed
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        cfg.energy = set.contains("energy");
        cfg.fleet = set.contains("fleet");
        cfg.readiness = set.contains("readiness");
        cfg.network = set.contains("network");
        if !cfg.energy && !cfg.fleet && !cfg.readiness && !cfg.network {
            return Self::default();
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_means_all() {
        let c = DashboardConfig::from_panels_csv("");
        assert!(c.energy && c.fleet && c.readiness && c.network);
    }

    #[test]
    fn subset() {
        let c = DashboardConfig::from_panels_csv("energy,fleet");
        assert!(c.energy && c.fleet);
        assert!(!c.readiness && !c.network);
    }
}
