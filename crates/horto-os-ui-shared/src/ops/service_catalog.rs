//! Full Horto service link catalog (display name + default port).
//!
//! Used by status-api Services list and by d3 when writing `service_links.env`.
//! Installation may skip stacks; the catalog still lists every known service.

use crate::stack_opts::StackOpts;

/// One known service link: UI name and default listen port on the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLinkDef {
    /// Display name (`Homepage`, `Dockge`, …).
    pub name: &'static str,
    /// Default host port when the service is not running / not remapped.
    pub port: u16,
}

/// Every known Horto service link (stable order).
pub const SERVICE_CATALOG: &[ServiceLinkDef] = &[
    ServiceLinkDef {
        name: "Homepage",
        port: 3021,
    },
    ServiceLinkDef {
        name: "Dockge",
        port: 5001,
    },
    ServiceLinkDef {
        name: "Cockpit",
        port: 9890,
    },
    ServiceLinkDef {
        name: "Open-WebUI",
        port: 3000,
    },
    ServiceLinkDef {
        name: "EVCC",
        port: 7070,
    },
    ServiceLinkDef {
        name: "Whisper",
        port: 8000,
    },
    ServiceLinkDef {
        name: "DeepSeek",
        port: 8001,
    },
    ServiceLinkDef {
        name: "Piper",
        port: 10200,
    },
    ServiceLinkDef {
        name: "OpenWakeWord",
        port: 10400,
    },
    ServiceLinkDef {
        name: "Status-API",
        port: 8787,
    },
    ServiceLinkDef {
        name: "MCP",
        port: 8790,
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

/// Normalize a service or container name for catalog matching.
#[must_use]
pub fn normalize_service_key(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// True when a Docker container/stack name matches a catalog service.
#[must_use]
pub fn container_matches_service(service_name: &str, names: &str, stack: Option<&str>) -> bool {
    let want = catalog_match_keys(service_name);
    if want.is_empty() {
        return false;
    }
    if let Some(stack) = stack {
        let sk = normalize_service_key(stack);
        if want.iter().any(|k| *k == sk) {
            return true;
        }
    }
    for part in names.split(',') {
        let key = normalize_service_key(part.trim().trim_start_matches('/'));
        if want
            .iter()
            .any(|k| *k == key || key.contains(k) || k.contains(&key))
        {
            return true;
        }
    }
    false
}

fn catalog_match_keys(service_name: &str) -> Vec<&'static str> {
    match normalize_service_key(service_name).as_str() {
        "homepage" => vec!["homepage"],
        "dockge" => vec!["dockge"],
        "cockpit" => vec!["cockpit"],
        "open-webui" | "openwebui" => vec!["open-webui", "openwebui"],
        "evcc" => vec!["evcc"],
        "whisper" => vec!["whisper", "whisper-cv"],
        "deepseek" => vec!["deepseek", "deepseek-npu"],
        "piper" => vec!["piper", "wyoming-piper"],
        "openwakeword" => vec!["openwakeword", "openwakeword-wyoming"],
        "status-api" | "statusapi" => {
            vec!["status-api", "horto-os-ui-status-api", "statusapi"]
        }
        "mcp" => vec!["mcp", "horto-os-ui-mcp"],
        _ => vec![],
    }
}

/// First host-published TCP port from a `docker ps` Ports column, if any.
///
/// Examples: `0.0.0.0:3021->3000/tcp`, `:::5001->5001/tcp`.
#[must_use]
pub fn host_port_from_docker_ports(ports_col: &str) -> Option<u16> {
    for chunk in ports_col.split(',') {
        let chunk = chunk.trim();
        if let Some((left, _right)) = chunk.split_once("->") {
            if let Some((_, host_port)) = left.rsplit_once(':') {
                let host_port = host_port.split('-').next().unwrap_or(host_port);
                if let Ok(p) = host_port.parse::<u16>() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Default port for a catalog service name, if known.
#[must_use]
pub fn default_port_for(name: &str) -> Option<u16> {
    let key = normalize_service_key(name);
    SERVICE_CATALOG
        .iter()
        .find(|s| normalize_service_key(s.name) == key)
        .map(|s| s.port)
}

/// [`StackOpts`] link fragments stay aligned with this catalog for selected stacks.
#[must_use]
pub fn stack_link_in_catalog(opts: StackOpts) -> bool {
    opts.selected().iter().all(|s| {
        let (name, port) = s.link.split_once(':').unwrap_or(("", ""));
        default_port_for(name).is_some_and(|p| port.parse::<u16>().ok() == Some(p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_links_value_is_full() {
        let v = service_links_value();
        assert!(v.contains("Homepage:3021"));
        assert!(v.contains("Dockge:5001"));
        assert!(v.contains("Cockpit:9890"));
        assert!(v.contains("Status-API:8787"));
        assert!(v.contains("MCP:8790"));
        assert!(v.contains("OpenWakeWord:10400"));
        assert_eq!(v.matches(',').count(), SERVICE_CATALOG.len() - 1);
    }

    #[test]
    fn docker_host_port_parses() {
        assert_eq!(
            host_port_from_docker_ports("0.0.0.0:3021->3000/tcp"),
            Some(3021)
        );
        assert_eq!(
            host_port_from_docker_ports(":::5001->5001/tcp, 0.0.0.0:5001->5001/tcp"),
            Some(5001)
        );
        assert_eq!(host_port_from_docker_ports("80/tcp"), None);
    }

    #[test]
    fn container_match_by_stack_and_name() {
        assert!(container_matches_service(
            "Homepage",
            "web",
            Some("homepage")
        ));
        assert!(container_matches_service("Dockge", "dockge", None));
        assert!(container_matches_service(
            "Whisper",
            "whisper-cv",
            Some("whisper-cv")
        ));
        assert!(!container_matches_service(
            "Homepage",
            "nginx",
            Some("other")
        ));
    }

    #[test]
    fn stack_opts_links_match_catalog() {
        let opts = StackOpts {
            dockge: true,
            open_webui: true,
            evcc: true,
            whisper: true,
            deepseek: true,
            piper: true,
            openwakeword: true,
        };
        assert!(stack_link_in_catalog(opts));
    }
}
