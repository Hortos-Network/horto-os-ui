//! Aggregate box status for CLI / TUI / status API.

use crate::context::HostContext;
use crate::embed;
use crate::kits::docker::{self, ContainerInfo};
use crate::kits::envfile;
use crate::ops::backup::{self, BackupStatus};
use crate::ops::catalog;
use crate::ops::doctor::{self, DoctorReport};
use crate::ops::host_metrics::{self, HostMetrics};
use crate::ops::leases::{self, LeaseEntry};
use crate::ops::service_catalog::{
    container_matches_service, host_port_from_docker_ports, normalize_service_key, SERVICE_CATALOG,
};
use crate::pipeline::{self, SetupKind};
use crate::resume::{self, StepStatus};
use crate::LONG_VERSION;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One service link derived from the full catalog, overlays, and probes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlInfo {
    /// Display name (e.g. `Homepage`).
    pub name: String,
    /// Absolute URL including scheme, host, and port.
    pub url: String,
    /// TCP probe on the box loopback for this link's port (`true` = accepting).
    #[serde(default)]
    pub up: bool,
    /// Short blurb for known Horto services when catalog matches.
    #[serde(default)]
    pub description: Option<String>,
}

/// One pipeline step row in a setup status report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStatusRow {
    /// Step id (`s1`, …).
    pub id: String,
    /// Human title from [`Step::title`](crate::step::Step::title).
    pub title: String,
    /// `pending` / `done` / `stale` / `failed`.
    pub status: String,
    /// Binary [`Step::step_version`](crate::step::Step::step_version).
    pub step_version: u32,
    /// Whether apply is destructive.
    pub destructive: bool,
    /// Whether a reboot is advised after this step.
    pub needs_reboot_after: bool,
}

/// Setup pipeline kind plus per-step status rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStatusReport {
    /// [`SetupKind::as_str`] value (`full` / `minimal`).
    pub kind: String,
    /// Ordered rows matching [`pipeline`](crate::pipeline::pipeline).
    pub steps: Vec<StepStatusRow>,
}

/// Full box snapshot: hostname, setup, doctor, backup, docker, leases, service URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxStatus {
    /// Tip long-version of the reporting binary (`0.1.0 (abc1234)`).
    #[serde(default)]
    pub cli_version: String,
    /// Box hostname from `/etc/hostname`, `hostname`, or `HOSTNAME`.
    pub hostname: String,
    /// Local host sensors (CPU, disk, OS, apt).
    #[serde(default)]
    pub host: HostMetrics,
    /// Resume-aware setup status for the requested kind.
    pub setup: SetupStatusReport,
    /// Local doctor checks.
    pub doctor: DoctorReport,
    /// Backup path presence / disk probe summary.
    pub backup: BackupStatus,
    /// Running docker containers (empty when docker is unavailable).
    pub containers: Vec<ContainerInfo>,
    /// DHCP lease entries when lease files exist.
    pub leases: Vec<LeaseEntry>,
    /// Service links with loopback TCP `up` probes.
    pub urls: Vec<UrlInfo>,
}

/// Build the full service catalog with overlays and loopback TCP probes.
///
/// Starts from [`SERVICE_CATALOG`]. On-box `service_links.env` may override ports by
/// name. Running Docker containers may override with the published host port.
/// `link_host` is normally the box hostname. Explicit `HOST=` in the env file wins.
#[must_use]
pub fn service_urls(ctx: &HostContext, link_host: &str) -> Vec<UrlInfo> {
    let containers = docker::list_containers().unwrap_or_default();
    service_urls_merged(ctx, link_host, &containers)
}

fn service_urls_merged(
    ctx: &HostContext,
    link_host: &str,
    containers: &[ContainerInfo],
) -> Vec<UrlInfo> {
    let map = load_service_links(ctx);
    let (scheme, host) = scheme_and_host(&map, link_host);
    let overrides = ports_from_links_field(map.get("LINKS").map_or("", String::as_str));
    let mut urls = Vec::with_capacity(SERVICE_CATALOG.len());
    for entry in SERVICE_CATALOG {
        let key = normalize_service_key(entry.name);
        let port = live_port_for_service(entry.name, containers)
            .or_else(|| overrides.get(&key).copied())
            .unwrap_or(entry.port);
        let mut info = UrlInfo {
            name: entry.name.to_owned(),
            url: format!("{scheme}://{host}:{port}"),
            up: false,
            description: catalog::describe_service(entry.name).map(str::to_owned),
        };
        info.up = tcp_port_open(port);
        urls.push(info);
    }
    urls
}

fn live_port_for_service(service_name: &str, containers: &[ContainerInfo]) -> Option<u16> {
    for c in containers {
        if container_matches_service(service_name, &c.names, c.stack.as_deref()) {
            if let Some(p) = host_port_from_docker_ports(&c.ports) {
                return Some(p);
            }
        }
    }
    None
}

fn ports_from_links_field(links: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    for entry in links.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((name, port_s)) = entry.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let Ok(port) = port_s.trim().parse::<u16>() else {
            continue;
        };
        if !name.is_empty() {
            out.insert(normalize_service_key(name), port);
        }
    }
    out
}

fn load_service_links(ctx: &HostContext) -> BTreeMap<String, String> {
    let path = ctx.paths.service_links_file();
    if path.is_file() {
        if let Ok(map) = envfile::load(&path) {
            return map;
        }
    }
    embed::get_str("config/service_links.env")
        .map(|raw| envfile::parse(&raw))
        .unwrap_or_default()
}

fn scheme_and_host<'a>(
    map: &'a BTreeMap<String, String>,
    link_host: &'a str,
) -> (&'a str, &'a str) {
    let scheme = map
        .get("SCHEME")
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http");
    let configured_host = map
        .get("HOST")
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let host = configured_host.unwrap_or_else(|| {
        let h = link_host.trim();
        if h.is_empty() || h.eq_ignore_ascii_case("unknown") {
            "localhost"
        } else {
            h
        }
    });
    (scheme, host)
}

fn tcp_port_open(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Duration;

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

/// Build resume-aware step status rows for `kind`.
///
/// Pending resume records fall back to [`Step::is_done`](crate::step::Step::is_done)
/// when the host already looks complete.
#[must_use]
pub fn setup_status(ctx: &HostContext, kind: SetupKind) -> SetupStatusReport {
    let state = resume::load(&ctx.paths.resume_file()).unwrap_or_default();
    let steps = pipeline::pipeline(kind)
        .iter()
        .map(|s| {
            let st = resume::status_for(&state, *s);
            let status = match st {
                StepStatus::Pending => {
                    if s.is_done(ctx) {
                        "done".to_string()
                    } else {
                        "pending".to_string()
                    }
                }
                StepStatus::Done => "done".into(),
                StepStatus::Stale => "stale".into(),
                StepStatus::Failed => "failed".into(),
            };
            StepStatusRow {
                id: s.id().into(),
                title: s.title().into(),
                status,
                step_version: s.step_version(),
                destructive: s.destructive(),
                needs_reboot_after: s.needs_reboot_after(),
            }
        })
        .collect();
    SetupStatusReport {
        kind: kind.as_str().into(),
        steps,
    }
}

/// Collect box status. Service links use the box hostname unless `HOST=` is set
/// in `service_links.env`.
#[must_use]
pub fn box_status(ctx: &HostContext, kind: SetupKind) -> BoxStatus {
    let hostname = fs_hostname().unwrap_or_else(|| "unknown".into());
    let urls = service_urls(ctx, &hostname);
    let mut containers = docker::list_containers().unwrap_or_default();
    for container in &mut containers {
        container.description =
            catalog::describe_container(&container.names, &container.image).map(str::to_owned);
    }
    BoxStatus {
        cli_version: LONG_VERSION.to_owned(),
        hostname,
        host: host_metrics::collect_host_metrics(),
        setup: setup_status(ctx, kind),
        doctor: doctor::doctor(ctx),
        backup: backup::backup_status(ctx),
        containers,
        leases: leases::read_leases(&ctx.paths.lease_file, &ctx.paths.leases_json()),
        urls,
    }
}

fn fs_hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| hostname_cmd().or_else(|| std::env::var("HOSTNAME").ok()))
}

fn hostname_cmd() -> Option<String> {
    let o = std::process::Command::new("hostname").output().ok()?;
    if o.status.success() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{live_port_for_service, ports_from_links_field, scheme_and_host};
    use crate::kits::docker::ContainerInfo;
    use crate::ops::service_catalog::SERVICE_CATALOG;
    use std::collections::BTreeMap;

    #[test]
    fn scheme_and_host_prefers_configured() {
        let mut map = BTreeMap::new();
        map.insert("SCHEME".into(), "https".into());
        map.insert("HOST".into(), "horto-box".into());
        let (scheme, host) = scheme_and_host(&map, "ignored");
        assert_eq!(scheme, "https");
        assert_eq!(host, "horto-box");
    }

    #[test]
    fn scheme_and_host_falls_back() {
        let map = BTreeMap::new();
        let (scheme, host) = scheme_and_host(&map, "my-box");
        assert_eq!(scheme, "http");
        assert_eq!(host, "my-box");
        let (_, host) = scheme_and_host(&map, "unknown");
        assert_eq!(host, "localhost");
    }

    #[test]
    fn ports_from_partial_links_still_keyed() {
        let m = ports_from_links_field("Homepage:3999,Cockpit:9890");
        assert_eq!(m.get("homepage"), Some(&3999));
        assert_eq!(m.get("cockpit"), Some(&9890));
        assert!(!m.contains_key("dockge"));
    }

    #[test]
    fn live_port_overrides_from_docker() {
        let containers = vec![ContainerInfo {
            id: "x".into(),
            names: "homepage".into(),
            image: "gethomepage/homepage".into(),
            status: "Up".into(),
            ports: "0.0.0.0:3999->3000/tcp".into(),
            stack: Some("homepage".into()),
            description: None,
        }];
        assert_eq!(live_port_for_service("Homepage", &containers), Some(3999));
        assert_eq!(live_port_for_service("Dockge", &containers), None);
    }

    #[test]
    fn disk_override_applies_without_live_docker() {
        use crate::context::{ApplyMode, HostContext};
        use crate::kits::envfile;
        use crate::paths::HostPaths;
        use crate::pipeline::SetupKind;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let paths = HostPaths {
            active_setup: tmp.path().join("active_setup"),
            backup: tmp.path().join("backup"),
            docker: tmp.path().join("docker"),
            etc: tmp.path().join("etc"),
            lease_file: tmp.path().join("leases"),
        };
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("SCHEME".into(), "http".into());
        map.insert("HOST".into(), "cov-box".into());
        map.insert("LINKS".into(), "Homepage:3999".into());
        envfile::write(&paths.service_links_file(), &map).unwrap();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        let urls = super::service_urls_merged(&ctx, "ignored", &[]);
        assert_eq!(urls.len(), SERVICE_CATALOG.len());
        let homepage = urls.iter().find(|u| u.name == "Homepage").unwrap();
        assert_eq!(homepage.url, "http://cov-box:3999");
        assert!(urls.iter().any(|u| u.name == "Dockge"));
    }
}
