//! Full Horto service link catalog (display name + default port).
//!
//! Used when writing `service_links.env` (d3) and as the documented defaults in
//! `assets/config/service_links.env`. Desktop Services merges its own copy.

/// One known service link: UI name and default listen port on the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLinkDef {
    /// Display name (`Homepage`, `Dockge`, …).
    pub name: &'static str,
    /// Default host port when the service is not running / not remapped.
    pub port: u16,
}

/// Every known Horto service link (alphabetical by display name).
pub const SERVICE_CATALOG: &[ServiceLinkDef] = &[
    ServiceLinkDef {
        name: "Cockpit",
        port: 9890,
    },
    ServiceLinkDef {
        name: "DeepSeek",
        port: 8001,
    },
    ServiceLinkDef {
        name: "Dockge",
        port: 5001,
    },
    ServiceLinkDef {
        name: "EVCC",
        port: 7070,
    },
    ServiceLinkDef {
        name: "Homepage",
        port: 3021,
    },
    ServiceLinkDef {
        name: "MCP",
        port: 8790,
    },
    ServiceLinkDef {
        name: "Open-WebUI",
        port: 3000,
    },
    ServiceLinkDef {
        name: "OpenWakeWord",
        port: 10400,
    },
    ServiceLinkDef {
        name: "Piper",
        port: 10200,
    },
    ServiceLinkDef {
        name: "Status-API",
        port: 8787,
    },
    ServiceLinkDef {
        name: "Whisper",
        port: 8000,
    },
];

/// `LINKS=` value for `service_links.env` (full catalog).
#[must_use]
pub fn service_links_value() -> String {
    SERVICE_CATALOG
        .iter()
        .map(|s| format!("{}:{}", s.name, s.port))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_links_value_is_full_and_alphabetical() {
        let v = service_links_value();
        assert!(v.starts_with("Cockpit:9890"));
        assert!(v.contains("Homepage:3021"));
        assert!(v.contains("Dockge:5001"));
        assert!(v.contains("Status-API:8787"));
        assert!(v.contains("MCP:8790"));
        assert!(v.ends_with("Whisper:8000"));
        assert_eq!(v.matches(',').count(), SERVICE_CATALOG.len() - 1);
        let names: Vec<_> = SERVICE_CATALOG.iter().map(|s| s.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }
}
