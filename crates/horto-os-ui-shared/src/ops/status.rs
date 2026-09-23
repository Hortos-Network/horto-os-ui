//! Aggregate box status for CLI / TUI / status API.

use crate::context::HostContext;
use crate::embed;
use crate::kits::docker::{self, ContainerInfo};
use crate::kits::envfile;
use crate::ops::backup::{self, BackupStatus};
use crate::ops::catalog;
use crate::ops::doctor::{self, DoctorReport};
use crate::ops::leases::{self, LeaseEntry};
use crate::pipeline::{self, SetupKind};
use crate::resume::{self, StepStatus};
use crate::LONG_VERSION;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One service link derived from `service_links.env` (or embedded defaults).
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

/// Build service links from `active_setup/service_links.env`, else embedded defaults.
///
/// `link_host` is normally the box hostname. Explicit `HOST=` in the env file wins.
/// Each link's [`UrlInfo::up`] is set by a short TCP probe to `127.0.0.1:<port>`.
#[must_use]
pub fn service_urls(ctx: &HostContext, link_host: &str) -> Vec<UrlInfo> {
    let mut urls = urls_from_map(&load_service_links(ctx), link_host);
    for url in &mut urls {
        url.up = port_from_url(&url.url).is_some_and(tcp_port_open);
        url.description = catalog::describe_service(&url.name).map(str::to_owned);
    }
    urls
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

fn urls_from_map(map: &BTreeMap<String, String>, link_host: &str) -> Vec<UrlInfo> {
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
    let links = map.get("LINKS").map_or("", String::as_str);
    parse_links(scheme, host, links)
}

fn parse_links(scheme: &str, host: &str, links: &str) -> Vec<UrlInfo> {
    links
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (name, port) = entry.split_once(':')?;
            let name = name.trim();
            let port = port.trim();
            if name.is_empty() || port.is_empty() {
                return None;
            }
            Some(UrlInfo {
                name: name.to_owned(),
                url: format!("{scheme}://{host}:{port}"),
                up: false,
                description: None,
            })
        })
        .collect()
}

fn port_from_url(url: &str) -> Option<u16> {
    let after_scheme = url.split("://").nth(1)?;
    let host_port = after_scheme.split('/').next()?;
    let (_, port) = host_port.rsplit_once(':')?;
    port.parse().ok()
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
    use super::{parse_links, urls_from_map};
    use std::collections::BTreeMap;

    #[test]
    fn parse_links_builds_urls() {
        let urls = parse_links("https", "box.local", "Homepage:3021, Dockge:5001");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].name, "Homepage");
        assert_eq!(urls[0].url, "https://box.local:3021");
        assert_eq!(urls[1].url, "https://box.local:5001");
    }

    #[test]
    fn urls_from_map_uses_configured_host_and_ports() {
        let mut map = BTreeMap::new();
        map.insert("SCHEME".into(), "http".into());
        map.insert("HOST".into(), "horto-box".into());
        map.insert("LINKS".into(), "Homepage:3021,Cockpit:9890".into());
        let urls = urls_from_map(&map, "ignored");
        assert_eq!(urls[0].url, "http://horto-box:3021");
        assert_eq!(urls[1].url, "http://horto-box:9890");
    }

    #[test]
    fn urls_from_map_falls_back_to_hostname() {
        let mut map = BTreeMap::new();
        map.insert("LINKS".into(), "Dockge:5001".into());
        let urls = urls_from_map(&map, "my-box");
        assert_eq!(urls[0].url, "http://my-box:5001");
    }

    #[test]
    fn urls_from_map_unknown_hostname_uses_localhost() {
        let mut map = BTreeMap::new();
        map.insert("LINKS".into(), "Dockge:5001".into());
        let urls = urls_from_map(&map, "unknown");
        assert_eq!(urls[0].url, "http://localhost:5001");
    }

    #[test]
    fn port_from_url_parses_http() {
        assert_eq!(super::port_from_url("http://deb:3021/"), Some(3021));
        assert_eq!(super::port_from_url("https://box.local:5001"), Some(5001));
        assert_eq!(super::port_from_url("http://box/"), None);
    }
}
