//! OpenSSH host target: config Host alias or `user@host`.
//!
//! Lists LAN names from `/etc/hosts`, plus OpenSSH `Host` aliases whose
//! `HostName` is private or already named in `/etc/hosts` (no internet aliases).

use crate::error::{HortoError, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Destination passed to `ssh` / `scp` as the remote operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    /// Raw value suitable for `ssh <spec>` (Host alias or `user@host`).
    pub raw: String,
}

/// A hostname known from `/etc/hosts` and/or OpenSSH config (plus localhost).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownRemoteHost {
    /// Name suitable as an OpenSSH target (`ssh <name>`) or Status API host.
    pub name: String,
    /// Present as a non-loopback entry in `/etc/hosts` (or synthetic localhost).
    pub in_hosts_file: bool,
    /// Present as a concrete `Host` alias in OpenSSH config (LAN-qualified).
    pub in_ssh_config: bool,
}

/// One OpenSSH `Host` alias with its effective `HostName` (defaults to the alias).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfigHost {
    /// `Host` token (no wildcards).
    pub alias: String,
    /// `HostName` value, or the alias when omitted.
    pub host_name: String,
}

/// Parse a non-empty Host alias or `user@host` string.
///
/// # Errors
///
/// Returns [`crate::HortoError::Message`] when the string is empty or only whitespace.
pub fn parse_host_spec(input: &str) -> Result<HostSpec> {
    let raw = input.trim().to_owned();
    if raw.is_empty() {
        return Err(HortoError::msg(
            "remote host is empty; pass an OpenSSH Host alias or user@host",
        ));
    }
    if raw.contains(char::is_whitespace) {
        return Err(HortoError::msg(format!(
            "remote host must not contain whitespace: {raw:?}"
        )));
    }
    Ok(HostSpec { raw })
}

/// Merge `/etc/hosts` LAN names with LAN-qualified OpenSSH `Host` aliases.
///
/// Always includes `localhost` first (this machine / Desktop local Status API).
/// Missing files yield empty LAN sides (not an error). Names are sorted after
/// localhost; duplicates keep both source flags.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when a readable file cannot be read as UTF-8 text.
pub fn list_known_remote_hosts() -> Result<Vec<KnownRemoteHost>> {
    list_known_remote_hosts_from(Path::new("/etc/hosts"), &ssh_config_paths())
}

/// Testable merge of hosts-file text and SSH config texts (LAN filter applied).
///
/// `localhost` is always first so Desktop can target this machine.
#[must_use]
pub fn merge_known_remote_hosts(
    hosts_file_text: &str,
    ssh_config_texts: &[&str],
) -> Vec<KnownRemoteHost> {
    let hosts_names = parse_hosts_file_names(hosts_file_text);
    let mut by_name: BTreeMap<String, KnownRemoteHost> = BTreeMap::new();
    for name in &hosts_names {
        by_name.insert(
            name.clone(),
            KnownRemoteHost {
                name: name.clone(),
                in_hosts_file: true,
                in_ssh_config: false,
            },
        );
    }
    for text in ssh_config_texts {
        for entry in parse_ssh_config_hosts(text) {
            if !ssh_host_is_lan(&entry, &hosts_names) {
                continue;
            }
            by_name
                .entry(entry.alias.clone())
                .and_modify(|h| h.in_ssh_config = true)
                .or_insert(KnownRemoteHost {
                    name: entry.alias,
                    in_hosts_file: false,
                    in_ssh_config: true,
                });
        }
    }
    with_localhost_first(by_name.into_values().collect())
}

/// Put `localhost` first for local Desktop targeting; drop duplicates.
fn with_localhost_first(mut hosts: Vec<KnownRemoteHost>) -> Vec<KnownRemoteHost> {
    hosts.retain(|h| h.name != "localhost");
    let mut out = Vec::with_capacity(hosts.len() + 1);
    out.push(KnownRemoteHost {
        name: "localhost".into(),
        in_hosts_file: true,
        in_ssh_config: false,
    });
    out.extend(hosts);
    out
}

/// Hostnames from `/etc/hosts` text (non-loopback, non-meta).
#[must_use]
pub fn parse_hosts_file_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(addr) = parts.next() else {
            continue;
        };
        if is_skipped_hosts_address(addr) {
            continue;
        }
        for name in parts {
            let name = name.trim();
            if name.is_empty() || is_skipped_hosts_name(name) {
                continue;
            }
            if !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// Concrete `Host` aliases from OpenSSH config text (no wildcards).
#[must_use]
pub fn parse_ssh_config_host_aliases(text: &str) -> Vec<String> {
    parse_ssh_config_hosts(text)
        .into_iter()
        .map(|h| h.alias)
        .collect()
}

/// Parse `Host` / `HostName` blocks from OpenSSH config text.
#[must_use]
pub fn parse_ssh_config_hosts(text: &str) -> Vec<SshConfigHost> {
    let mut out = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut host_name: Option<String> = None;

    let flush = |current: &mut Vec<String>, host_name: &mut Option<String>, out: &mut Vec<_>| {
        if current.is_empty() {
            *host_name = None;
            return;
        }
        let hn = host_name.take();
        for alias in current.drain(..) {
            let resolved = hn.clone().unwrap_or_else(|| alias.clone());
            out.push(SshConfigHost {
                alias,
                host_name: resolved,
            });
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };
        if keyword.eq_ignore_ascii_case("host") {
            flush(&mut current, &mut host_name, &mut out);
            for token in parts {
                if token.contains('*') || token.contains('?') || token.starts_with('!') {
                    continue;
                }
                if !current.iter().any(|n| n == token) {
                    current.push(token.to_owned());
                }
            }
            continue;
        }
        if keyword.eq_ignore_ascii_case("hostname") {
            if let Some(value) = parts.next() {
                host_name = Some(value.to_owned());
            }
        }
    }
    flush(&mut current, &mut host_name, &mut out);
    out
}

/// True when an SSH config entry targets the LAN (private IP or `/etc/hosts` name).
#[must_use]
pub fn ssh_host_is_lan(entry: &SshConfigHost, hosts_file_names: &[String]) -> bool {
    if hosts_file_names
        .iter()
        .any(|n| n == &entry.alias || n == &entry.host_name)
    {
        return true;
    }
    is_private_or_link_local_ip(&entry.host_name)
}

fn list_known_remote_hosts_from(
    hosts_path: &Path,
    ssh_paths: &[PathBuf],
) -> Result<Vec<KnownRemoteHost>> {
    let hosts_text = read_optional_text(hosts_path)?;
    let mut ssh_texts = Vec::new();
    let mut ssh_refs: Vec<&str> = Vec::new();
    for path in ssh_paths {
        let text = read_optional_text(path)?;
        if !text.is_empty() {
            ssh_texts.push(text);
        }
    }
    for text in &ssh_texts {
        ssh_refs.push(text.as_str());
    }
    Ok(merge_known_remote_hosts(&hosts_text, &ssh_refs))
}

fn read_optional_text(path: &Path) -> Result<String> {
    if !path.is_file() {
        return Ok(String::new());
    }
    fs::read_to_string(path)
        .map_err(|e| HortoError::msg(format!("failed to read {}: {e}", path.display())))
}

fn ssh_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let user = PathBuf::from(home).join(".ssh").join("config");
        if user.is_file() {
            paths.push(user);
        }
    }
    let system = PathBuf::from("/etc/ssh/ssh_config");
    if system.is_file() {
        paths.push(system);
    }
    paths
}

fn is_skipped_hosts_address(addr: &str) -> bool {
    let lower = addr.to_ascii_lowercase();
    lower == "::1"
        || lower.starts_with("127.")
        || lower.starts_with("fe80:")
        || lower.starts_with("ff0")
}

fn is_skipped_hosts_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "localhost" | "ip6-localhost" | "ip6-loopback" | "ip6-allnodes" | "ip6-allrouters"
    )
}

fn is_private_or_link_local_ip(host: &str) -> bool {
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            // Unique local fc00::/7 or link-local fe80::/10
            (octets[0] & 0xfe) == 0xfc || (octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_alias_and_user_host() {
        assert_eq!(parse_host_spec(" horto-box ").unwrap().raw, "horto-box");
        assert_eq!(
            parse_host_spec("greg@192.168.1.10").unwrap().raw,
            "greg@192.168.1.10"
        );
    }

    #[test]
    fn rejects_empty_or_spaces() {
        assert!(parse_host_spec("").is_err());
        assert!(parse_host_spec("  ").is_err());
        assert!(parse_host_spec("bad host").is_err());
    }

    #[test]
    fn parses_hosts_file_keeps_lan_drops_loopback() {
        let text = "\
127.0.0.1\tlocalhost
127.0.1.1\tdeb
#10.20.33.233\tcasper-box
192.168.0.3\tdebby.local
192.168.0.242\thorto
::1\tlocalhost ip6-localhost ip6-loopback
fe80::1\tlinklocal
";
        let names = parse_hosts_file_names(text);
        assert_eq!(names, vec!["debby.local".to_owned(), "horto".to_owned()]);
    }

    #[test]
    fn parses_ssh_config_aliases_skips_wildcards() {
        let text = "\
Host *
  IdentityFile ~/.ssh/id_ed25519

Host horto
  HostName horto
  User greg

Host verq_pre_prod box?
  User groussac
";
        let names = parse_ssh_config_host_aliases(text);
        assert_eq!(names, vec!["horto".to_owned(), "verq_pre_prod".to_owned()]);
    }

    #[test]
    fn merge_keeps_lan_drops_internet_ssh_alias() {
        let hosts = "192.168.0.242 horto\n192.168.0.3 debby.local\n";
        let ssh = "\
Host horto
  HostName horto
  User greg

Host verq_pre_prod
  HostName verq.example.com
  User groussac

Host lab
  HostName 10.0.0.5
";
        let list = merge_known_remote_hosts(hosts, &[ssh]);
        let names: Vec<_> = list.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["localhost", "debby.local", "horto", "lab"]);
        assert!(!names.contains(&"verq_pre_prod"));
        let horto = list.iter().find(|h| h.name == "horto").unwrap();
        assert!(horto.in_hosts_file && horto.in_ssh_config);
        let lab = list.iter().find(|h| h.name == "lab").unwrap();
        assert!(!lab.in_hosts_file && lab.in_ssh_config);
        assert_eq!(list[0].name, "localhost");
    }

    #[test]
    fn merge_always_includes_localhost_first() {
        let list = merge_known_remote_hosts("", &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "localhost");
    }

    #[test]
    fn list_known_remote_hosts_from_temp_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hosts = tmp.path().join("hosts");
        let ssh = tmp.path().join("config");
        fs::write(&hosts, "192.168.0.10 box-a\n").unwrap();
        fs::write(
            &ssh,
            "Host box-a\n  HostName box-a\n\nHost lab\n  HostName 10.0.0.2\n",
        )
        .unwrap();
        let list = list_known_remote_hosts_from(&hosts, &[ssh]).unwrap();
        let names: Vec<_> = list.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["localhost", "box-a", "lab"]);
        let missing = tmp.path().join("missing");
        let empty = list_known_remote_hosts_from(&missing, &[]).unwrap();
        assert_eq!(empty[0].name, "localhost");
    }

    #[test]
    fn private_ip_and_skip_helpers() {
        assert!(is_private_or_link_local_ip("10.0.0.1"));
        assert!(is_private_or_link_local_ip("192.168.1.1"));
        assert!(!is_private_or_link_local_ip("8.8.8.8"));
        assert!(!is_private_or_link_local_ip("not-an-ip"));
        assert!(is_skipped_hosts_address("127.0.0.1"));
        assert!(is_skipped_hosts_address("::1"));
        assert!(is_skipped_hosts_name("localhost"));
        assert!(!is_skipped_hosts_name("horto"));
        let lan = SshConfigHost {
            alias: "lab".into(),
            host_name: "10.1.2.3".into(),
        };
        assert!(ssh_host_is_lan(&lan, &[]));
        let wan = SshConfigHost {
            alias: "edge".into(),
            host_name: "edge.example.com".into(),
        };
        assert!(!ssh_host_is_lan(&wan, &[]));
        assert!(ssh_host_is_lan(&wan, &["edge".into()]));
    }
}
