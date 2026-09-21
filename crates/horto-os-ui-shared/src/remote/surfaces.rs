//! Cross-surface probes: SSH, box CLI, status-api, MCP (PC + box).
//!
//! One report powers TUI tabs, `horto-os-ui surfaces`, and Desktop Connection.

use super::process::{ProcessRunner, StdioMode};
use super::runner::{
    api_token_config_path, classify_ssh_failure, probe_remote_cli, session_from,
    RemoteBoxCliStatus, RemoteOptions,
};
use crate::error::Result;
use crate::LONG_VERSION;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::io::{Read, Write as IoWrite};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_API_PORT: u16 = 8787;
const MCP_HTTP_PORT: u16 = 8790;

/// Full surface probe for tip + box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceProbeReport {
    /// Tip CLI long version (`local=`).
    pub local_version: String,
    /// OpenSSH access to the box (or embedded N/A).
    pub ssh: SshSurfaceProbe,
    /// Box CLI binary / version.
    pub cli: CliSurfaceProbe,
    /// Status API `:8787`.
    pub api: ApiSurfaceProbe,
    /// MCP on the tip (PC): Docker / binary / HTTP / unit / API health.
    pub mcp_pc: McpHostProbe,
    /// MCP on the box: Docker / binary / HTTP / unit / API health.
    pub mcp_box: McpHostProbe,
}

/// SSH reachability / key auth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSurfaceProbe {
    /// Host string used for OpenSSH.
    pub host: String,
    /// Operator label: `ok`, `auth failed`, `unreachable`, `n/a`.
    pub status: String,
    /// True when `BatchMode` key login works.
    pub key_ok: bool,
}

/// Box CLI probe row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliSurfaceProbe {
    /// After probe: missing / auth failed / unreachable / Found(version).
    pub status: RemoteBoxCliStatus,
    /// Normalized version when found.
    pub version: Option<String>,
    /// Matches tip [`LONG_VERSION`].
    pub current: bool,
}

/// Status-api HTTP (+ optional unit via SSH).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiSurfaceProbe {
    /// Base URL probed (`http://host:8787`).
    pub url: String,
    /// `ok` / `unreachable` / `http <code>` / `auth required`.
    pub health: String,
    /// `/v1/status` label when attempted.
    pub status: String,
    /// Local token file present (never the secret).
    pub local_token: bool,
    /// `systemctl is-active` when SSH worked; empty otherwise.
    pub unit: String,
}

/// MCP readiness on one host (PC or box): stdio runtime + HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpHostProbe {
    /// Docker image tag when present (`HORTO_MCP_IMAGE` or default).
    pub docker: Option<String>,
    /// Path to `horto-os-ui-mcp` when found.
    pub binary: Option<String>,
    /// `http://host:8790`.
    pub http_url: String,
    /// HTTP reachability label for [`Self::http_url`].
    pub http_reach: String,
    /// Unit/process hint when available.
    pub unit: String,
    /// Status-api `/health` on this host (`:8787`).
    pub api_health: String,
}

/// Default MCP image tag (matches `docker/cursor-mcp-stdio.sh`).
const DEFAULT_MCP_IMAGE: &str = "horto-os-ui-mcp:local";

/// Read local tip bearer when present (hex only).
#[must_use]
pub fn read_local_api_token() -> Option<String> {
    let path = api_token_config_path();
    let raw = std::fs::read_to_string(path).ok()?;
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let hex = line.strip_prefix("HORTO_API_TOKEN=").unwrap_or(line).trim();
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_owned())
}

/// Path to the local tip API token file (UI display only).
#[must_use]
pub fn local_api_token_path() -> PathBuf {
    api_token_config_path()
}

/// Probe all surfaces for a remote host, or loopback when `embedded`.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the SSH host string cannot be parsed.
pub fn probe_surfaces(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
) -> Result<SurfaceProbeReport> {
    let host = if embedded {
        "127.0.0.1".to_owned()
    } else {
        opts.host.clone()
    };

    let ssh = if embedded {
        SshSurfaceProbe {
            host: host.clone(),
            status: "n/a".into(),
            key_ok: false,
        }
    } else {
        probe_ssh_access(runner, opts)?
    };

    let cli = probe_cli_row(runner, opts, embedded, &ssh)?;
    let token = read_local_api_token();
    let api = probe_api_row(runner, opts, &host, embedded, &ssh, token.as_deref());
    let mcp_pc = probe_mcp_pc(runner, &host, opts.bin_dir.as_deref());
    let mcp_box = probe_mcp_box(runner, opts, &host, embedded, &ssh);

    Ok(SurfaceProbeReport {
        local_version: LONG_VERSION.to_owned(),
        ssh,
        cli,
        api,
        mcp_pc,
        mcp_box,
    })
}

/// Probe SSH only (`BatchMode`).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the host string cannot be parsed.
pub fn probe_ssh_surface(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
) -> Result<SshSurfaceProbe> {
    if embedded {
        return Ok(SshSurfaceProbe {
            host: "127.0.0.1".into(),
            status: "n/a".into(),
            key_ok: false,
        });
    }
    probe_ssh_access(runner, opts)
}

/// Probe box CLI only.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH host parsing or remote CLI probe fails.
pub fn probe_cli_surface(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
) -> Result<CliSurfaceProbe> {
    let ssh = probe_ssh_surface(runner, opts, embedded)?;
    probe_cli_row(runner, opts, embedded, &ssh)
}

/// Probe status-api only.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the SSH host string cannot be parsed.
pub fn probe_api_surface(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
) -> Result<ApiSurfaceProbe> {
    let host = if embedded {
        "127.0.0.1".to_owned()
    } else {
        opts.host.clone()
    };
    let ssh = probe_ssh_surface(runner, opts, embedded)?;
    let token = read_local_api_token();
    Ok(probe_api_row(
        runner,
        opts,
        &host,
        embedded,
        &ssh,
        token.as_deref(),
    ))
}

/// Probe MCP PC + box only.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the SSH host string cannot be parsed.
pub fn probe_mcp_surface(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
) -> Result<(McpHostProbe, McpHostProbe)> {
    let host = if embedded {
        "127.0.0.1".to_owned()
    } else {
        opts.host.clone()
    };
    let ssh = probe_ssh_surface(runner, opts, embedded)?;
    Ok((
        probe_mcp_pc(runner, &host, opts.bin_dir.as_deref()),
        probe_mcp_box(runner, opts, &host, embedded, &ssh),
    ))
}

/// Human-readable multi-line report (CLI `surfaces`).
#[must_use]
pub fn format_surfaces_report(report: &SurfaceProbeReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "local={}", report.local_version);
    let _ = writeln!(
        out,
        "ssh={} key_ok={}",
        report.ssh.status, report.ssh.key_ok
    );
    let _ = writeln!(
        out,
        "cli={} current={}",
        report.cli.status.as_label(),
        report.cli.current
    );
    let _ = writeln!(
        out,
        "api={} health={} status={} token_file={} unit={}",
        report.api.url,
        report.api.health,
        report.api.status,
        report.api.local_token,
        empty_dash(&report.api.unit)
    );
    out.push_str(&format_mcp_host_line("mcp_pc", &report.mcp_pc));
    out.push_str(&format_mcp_host_line("mcp_box", &report.mcp_box));
    out
}

fn format_mcp_host_line(prefix: &str, p: &McpHostProbe) -> String {
    format!(
        "{prefix} docker={} binary={} http={} reach={} unit={} api_health={}\n",
        p.docker.as_deref().unwrap_or("missing"),
        p.binary.as_deref().unwrap_or("missing"),
        p.http_url,
        p.http_reach,
        empty_dash(&p.unit),
        p.api_health
    )
}

const fn empty_dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

fn probe_cli_row(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    embedded: bool,
    ssh: &SshSurfaceProbe,
) -> Result<CliSurfaceProbe> {
    if embedded {
        return Ok(CliSurfaceProbe {
            status: RemoteBoxCliStatus::Found(LONG_VERSION.to_owned()),
            version: Some(LONG_VERSION.to_owned()),
            current: true,
        });
    }
    if ssh.status == "ok" || ssh.key_ok {
        let session = session_from(opts)?;
        let probe = probe_remote_cli(runner, &session, opts)?;
        return Ok(CliSurfaceProbe {
            status: probe.status,
            version: probe.version,
            current: probe.current,
        });
    }
    Ok(CliSurfaceProbe {
        status: if ssh.status == "auth failed" {
            RemoteBoxCliStatus::AuthFailed
        } else {
            RemoteBoxCliStatus::Unreachable
        },
        version: None,
        current: false,
    })
}

/// `BatchMode` SSH `true` against the box (no password prompt).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the host string cannot be parsed.
pub fn probe_ssh_access(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
) -> Result<SshSurfaceProbe> {
    let session = session_from(opts)?;
    let pairs = session.env.as_pairs();
    let env: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut args = Vec::new();
    if let Some(cfg) = &session.config_file {
        args.push("-F".to_owned());
        args.push(cfg.display().to_string());
    }
    args.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        session.host.raw,
        "true".into(),
    ]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = runner.run("ssh", &refs, &env, StdioMode::Capture)?;
    if out.success() {
        return Ok(SshSurfaceProbe {
            host: opts.host.clone(),
            status: "ok".into(),
            key_ok: true,
        });
    }
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim()
    } else {
        out.stderr.trim()
    };
    let classified = classify_ssh_failure(&format!("exit {}: {detail}", out.status));
    let status = match classified {
        RemoteBoxCliStatus::AuthFailed => "auth failed",
        _ => "unreachable",
    };
    Ok(SshSurfaceProbe {
        host: opts.host.clone(),
        status: status.into(),
        key_ok: false,
    })
}

fn probe_api_row(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    host: &str,
    embedded: bool,
    ssh: &SshSurfaceProbe,
    token: Option<&str>,
) -> ApiSurfaceProbe {
    let url = format!("http://{host}:{STATUS_API_PORT}");
    let health = http_get_label(host, STATUS_API_PORT, "/health", None);
    let status = if health == "ok" {
        http_get_label(host, STATUS_API_PORT, "/v1/status", token)
    } else {
        "skipped".into()
    };
    let unit = if !embedded && (ssh.status == "ok" || ssh.key_ok) {
        ssh_systemctl_active(runner, opts, "horto-os-ui-status-api.service")
    } else {
        String::new()
    };
    ApiSurfaceProbe {
        url,
        health,
        status,
        local_token: token.is_some(),
        unit,
    }
}

fn probe_mcp_pc(runner: &dyn ProcessRunner, host: &str, bin_dir: Option<&Path>) -> McpHostProbe {
    let http_url = format!("http://{host}:{MCP_HTTP_PORT}");
    McpHostProbe {
        docker: find_mcp_docker(runner),
        binary: find_mcp_binary(bin_dir),
        http_url,
        http_reach: http_get_label(host, MCP_HTTP_PORT, "/", None),
        unit: local_pgrep_mcp(runner),
        api_health: http_get_label(host, STATUS_API_PORT, "/health", None),
    }
}

fn probe_mcp_box(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    host: &str,
    embedded: bool,
    ssh: &SshSurfaceProbe,
) -> McpHostProbe {
    let http_url = format!("http://{host}:{MCP_HTTP_PORT}");
    let http_reach = http_get_label(host, MCP_HTTP_PORT, "/", None);
    let api_health = http_get_label(host, STATUS_API_PORT, "/health", None);
    let ssh_ok = !embedded && (ssh.status == "ok" || ssh.key_ok);
    let (docker, binary, unit) = if embedded {
        (
            find_mcp_docker(runner),
            find_mcp_binary(opts.bin_dir.as_deref()),
            local_pgrep_mcp(runner),
        )
    } else if ssh_ok {
        let unit = {
            let active = ssh_systemctl_active(runner, opts, "horto-os-ui-mcp.service");
            if active.is_empty() || active == "inactive" || active == "unknown" {
                ssh_pgrep_mcp(runner, opts)
            } else {
                active
            }
        };
        let binary = ssh_mcp_binary(runner, opts);
        let docker = ssh_mcp_docker(runner, opts);
        (docker, binary, unit)
    } else {
        (None, None, String::new())
    };
    McpHostProbe {
        docker,
        binary,
        http_url,
        http_reach,
        unit,
        api_health,
    }
}

fn mcp_image_name() -> String {
    std::env::var("HORTO_MCP_IMAGE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MCP_IMAGE.to_owned())
}

fn find_mcp_docker(runner: &dyn ProcessRunner) -> Option<String> {
    let image = mcp_image_name();
    match runner.run(
        "docker",
        &["image", "inspect", "--format", "{{.Id}}", image.as_str()],
        &[],
        StdioMode::Capture,
    ) {
        Ok(out) if out.success() && !out.stdout.trim().is_empty() => Some(image),
        _ => None,
    }
}

fn find_mcp_binary(bin_dir: Option<&Path>) -> Option<String> {
    if let Some(dir) = bin_dir {
        let p = dir.join("horto-os-ui-mcp");
        if p.is_file() {
            return Some(p.display().to_string());
        }
    }
    which::which("horto-os-ui-mcp")
        .ok()
        .map(|p| p.display().to_string())
}

fn local_pgrep_mcp(runner: &dyn ProcessRunner) -> String {
    match runner.run("pgrep", &["-a", "horto-os-ui-mcp"], &[], StdioMode::Capture) {
        Ok(out) if out.success() && !out.stdout.trim().is_empty() => "process".into(),
        _ => String::new(),
    }
}

fn ssh_mcp_binary(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Option<String> {
    let Ok(session) = session_from(opts) else {
        return None;
    };
    let cmd = "command -v horto-os-ui-mcp 2>/dev/null || true";
    match session.exec(runner, cmd, StdioMode::Capture) {
        Ok(out) => {
            let path = out.stdout.lines().next().unwrap_or("").trim();
            if path.is_empty() {
                None
            } else {
                Some(path.to_owned())
            }
        }
        Err(_) => None,
    }
}

fn ssh_mcp_docker(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Option<String> {
    let Ok(session) = session_from(opts) else {
        return None;
    };
    let image = mcp_image_name();
    let quoted = image.replace('\'', "'\\''");
    let cmd = format!("docker image inspect --format '{{{{.Id}}}}' '{quoted}' 2>/dev/null || true");
    match session.exec(runner, &cmd, StdioMode::Capture) {
        Ok(out) if !out.stdout.trim().is_empty() => Some(image),
        _ => None,
    }
}

fn ssh_systemctl_active(runner: &dyn ProcessRunner, opts: &RemoteOptions, unit: &str) -> String {
    let Ok(session) = session_from(opts) else {
        return String::new();
    };
    let cmd = format!("systemctl is-active {unit} 2>/dev/null || true");
    match session.exec(runner, &cmd, StdioMode::Capture) {
        Ok(out) => out
            .stdout
            .lines()
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_owned(),
        Err(_) => String::new(),
    }
}

fn ssh_pgrep_mcp(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> String {
    let Ok(session) = session_from(opts) else {
        return String::new();
    };
    let cmd = "pgrep -a horto-os-ui-mcp 2>/dev/null | head -1 || true";
    match session.exec(runner, cmd, StdioMode::Capture) {
        Ok(out) if !out.stdout.trim().is_empty() => "process".into(),
        _ => "inactive".into(),
    }
}

/// Minimal HTTP/1.0 GET label for probes (no extra HTTP crate).
#[must_use]
pub fn http_get_label(host: &str, port: u16, path: &str, bearer: Option<&str>) -> String {
    let addr = format!("{host}:{port}");
    tcp_http_get(&addr, host, path, bearer)
}

fn tcp_http_get(addr: &str, host: &str, path: &str, bearer: Option<&str>) -> String {
    let Ok(mut addrs) = std::net::ToSocketAddrs::to_socket_addrs(addr) else {
        return "unreachable".into();
    };
    let Some(sock) = addrs.next() else {
        return "unreachable".into();
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&sock, HTTP_TIMEOUT) else {
        return "unreachable".into();
    };
    finish_http_get(&mut stream, host, path, bearer)
}

fn finish_http_get(stream: &mut TcpStream, host: &str, path: &str, bearer: Option<&str>) -> String {
    let _ = stream.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_TIMEOUT));
    let auth = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n{auth}\r\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return "unreachable".into();
    }
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let status_line = text.lines().next().unwrap_or("");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok());
    match code {
        Some(200) => "ok".into(),
        Some(401 | 403) => "auth required".into(),
        Some(c) => format!("http {c}"),
        None if text.is_empty() => "unreachable".into(),
        None => "ok".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::process::CommandOutput;
    use crate::remote::process::ScriptedRunner;
    use std::io::Write;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    fn with_xdg_config<R>(tmp: &tempfile::TempDir, f: impl FnOnce() -> R) -> R {
        let _guard = crate::remote::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        // SAFETY: serialized by ENV_LOCK for remote tests.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        }
        let out = f();
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("XDG_CONFIG_HOME", v);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
        out
    }

    #[test]
    fn with_xdg_config_restores_absent_var() {
        let tmp = tempfile::TempDir::new().unwrap();
        let guard = crate::remote::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        drop(guard);
        with_xdg_config(&tmp, || {
            assert!(std::env::var("XDG_CONFIG_HOME").is_ok());
        });
        assert!(std::env::var("XDG_CONFIG_HOME").is_err());
    }

    #[test]
    fn format_surfaces_report_includes_local_and_rows() {
        let report = SurfaceProbeReport {
            local_version: "0.1.0 (deadbeef)".into(),
            ssh: SshSurfaceProbe {
                host: "horto".into(),
                status: "ok".into(),
                key_ok: true,
            },
            cli: CliSurfaceProbe {
                status: RemoteBoxCliStatus::Found("0.1.0 (deadbeef)".into()),
                version: Some("0.1.0 (deadbeef)".into()),
                current: true,
            },
            api: ApiSurfaceProbe {
                url: "http://horto:8787".into(),
                health: "ok".into(),
                status: "ok".into(),
                local_token: true,
                unit: "active".into(),
            },
            mcp_pc: McpHostProbe {
                docker: Some("horto-os-ui-mcp:local".into()),
                binary: Some("/usr/bin/horto-os-ui-mcp".into()),
                http_url: "http://horto:8790".into(),
                http_reach: "unreachable".into(),
                unit: String::new(),
                api_health: "ok".into(),
            },
            mcp_box: McpHostProbe {
                docker: None,
                binary: None,
                http_url: "http://horto:8790".into(),
                http_reach: "unreachable".into(),
                unit: "inactive".into(),
                api_health: "ok".into(),
            },
        };
        let text = format_surfaces_report(&report);
        assert!(text.contains("local=0.1.0 (deadbeef)"));
        assert!(text.contains("ssh=ok"));
        assert!(text.contains("cli=0.1.0 (deadbeef)"));
        assert!(text.contains("api=http://horto:8787"));
        assert!(text.contains("mcp_pc"));
        assert!(text.contains("mcp_box"));
        assert!(text.contains("docker=horto-os-ui-mcp:local"));
    }

    #[test]
    fn probe_ssh_access_ok_and_auth_failed() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let p = probe_ssh_access(&runner, &opts).unwrap();
        assert_eq!(p.status, "ok");
        assert!(p.key_ok);

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
        let p2 = probe_ssh_access(&runner2, &opts).unwrap();
        assert_eq!(p2.status, "auth failed");
        assert!(!p2.key_ok);
    }

    #[test]
    fn probe_ssh_access_unreachable() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection refused"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let p = probe_ssh_access(&runner, &opts).unwrap();
        assert_eq!(p.status, "unreachable");
    }

    #[test]
    fn probe_surfaces_embedded_skips_ssh() {
        let runner = ScriptedRunner::default();
        let opts = RemoteOptions::default();
        let report = probe_surfaces(&runner, &opts, true).unwrap();
        assert_eq!(report.ssh.status, "n/a");
        assert!(report.cli.current);
        assert!(report.mcp_pc.http_url.starts_with("http://127.0.0.1:8790"));
        assert!(report.mcp_box.http_url.starts_with("http://127.0.0.1:8790"));
    }

    #[test]
    fn probe_surfaces_remote_auth_maps_cli() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let report = probe_surfaces(&runner, &opts, false).unwrap();
        assert_eq!(report.ssh.status, "auth failed");
        assert_eq!(report.cli.status, RemoteBoxCliStatus::AuthFailed);
    }

    #[test]
    fn http_get_label_ok_from_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok");
        });
        assert_eq!(http_get_label("127.0.0.1", port, "/health", None), "ok");
    }

    #[test]
    fn http_get_label_auth_required() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.0 401 Unauthorized\r\n\r\n");
        });
        assert_eq!(
            http_get_label("127.0.0.1", port, "/v1/status", Some("tok")),
            "auth required"
        );
    }

    #[test]
    fn http_get_label_unreachable_port() {
        assert_eq!(
            http_get_label("127.0.0.1", 1, "/health", None),
            "unreachable"
        );
    }

    #[test]
    fn read_local_api_token_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        with_xdg_config(&tmp, || {
            let path = api_token_config_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"aabbccdd\n").unwrap();
            assert_eq!(read_local_api_token().as_deref(), Some("aabbccdd"));
        });
    }

    #[test]
    fn find_mcp_binary_in_bin_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bin = tmp.path().join("horto-os-ui-mcp");
        std::fs::write(&bin, b"#!/bin/true\n").unwrap();
        assert_eq!(
            find_mcp_binary(Some(tmp.path())).as_deref(),
            Some(bin.to_str().unwrap())
        );
        assert!(find_mcp_binary(Some(tmp.path().join("nope").as_path())).is_none());
    }

    #[test]
    fn probe_surfaces_remote_ok_runs_cli_probe() {
        let runner = ScriptedRunner::default();
        // ssh BatchMode true
        runner.push("ssh", ScriptedRunner::ok(""));
        // probe_remote_cli: install then agent
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
        // api unit
        runner.push("ssh", ScriptedRunner::ok("active\n"));
        // mcp unit
        runner.push("ssh", ScriptedRunner::ok("inactive\n"));
        // mcp pgrep
        runner.push("ssh", ScriptedRunner::ok(""));
        // mcp binary
        runner.push("ssh", ScriptedRunner::ok(""));
        // mcp docker
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let report = probe_surfaces(&runner, &opts, false).unwrap();
        assert_eq!(report.ssh.status, "ok");
        assert!(report.cli.current);
        assert_eq!(report.api.unit, "active");
    }

    #[test]
    fn surface_probe_report_json_roundtrip() {
        let report = SurfaceProbeReport {
            local_version: "0.1.0".into(),
            ssh: SshSurfaceProbe {
                host: "h".into(),
                status: "ok".into(),
                key_ok: true,
            },
            cli: CliSurfaceProbe {
                status: RemoteBoxCliStatus::Missing,
                version: None,
                current: false,
            },
            api: ApiSurfaceProbe {
                url: "http://h:8787".into(),
                health: "unreachable".into(),
                status: "skipped".into(),
                local_token: false,
                unit: String::new(),
            },
            mcp_pc: McpHostProbe {
                docker: None,
                binary: None,
                http_url: "http://h:8790".into(),
                http_reach: "unreachable".into(),
                unit: String::new(),
                api_health: "unreachable".into(),
            },
            mcp_box: McpHostProbe {
                docker: None,
                binary: None,
                http_url: "http://h:8790".into(),
                http_reach: "unreachable".into(),
                unit: String::new(),
                api_health: "unreachable".into(),
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: SurfaceProbeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
        let text = format_surfaces_report(&report);
        assert!(text.contains("unit=-"));
        assert!(text.contains("binary=missing"));
        assert!(text.contains("docker=missing"));
    }

    #[test]
    fn http_get_label_other_status_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.0 503 Service Unavailable\r\n\r\n");
        });
        assert_eq!(
            http_get_label("127.0.0.1", port, "/health", None),
            "http 503"
        );
    }

    #[test]
    fn local_api_token_path_is_under_config() {
        let p = local_api_token_path();
        assert!(p.to_string_lossy().contains("horto-os-ui"));
    }

    #[test]
    fn probe_surfaces_remote_ok_mcp_process_fallback() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
        runner.push("ssh", ScriptedRunner::ok("active\n"));
        runner.push("ssh", ScriptedRunner::ok("inactive\n"));
        runner.push("ssh", ScriptedRunner::ok("1234 horto-os-ui-mcp\n"));
        runner.push(
            "ssh",
            ScriptedRunner::ok("/usr/local/bin/horto-os-ui-mcp\n"),
        );
        runner.push("ssh", ScriptedRunner::ok("sha256:deadbeef\n"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let report = probe_surfaces(&runner, &opts, false).unwrap();
        assert_eq!(report.mcp_box.unit, "process");
        assert_eq!(
            report.mcp_box.binary.as_deref(),
            Some("/usr/local/bin/horto-os-ui-mcp")
        );
        assert_eq!(
            report.mcp_box.docker.as_deref(),
            Some("horto-os-ui-mcp:local")
        );
    }

    #[test]
    fn probe_surfaces_remote_unreachable_maps_cli() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection refused"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let report = probe_surfaces(&runner, &opts, false).unwrap();
        assert_eq!(report.ssh.status, "unreachable");
        assert_eq!(report.cli.status, RemoteBoxCliStatus::Unreachable);
    }

    #[test]
    fn probe_ssh_access_uses_config_file_flag() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ssh_config_file: Some(PathBuf::from("/tmp/horto-test-ssh-config")),
            ..RemoteOptions::default()
        };
        let p = probe_ssh_access(&runner, &opts).unwrap();
        assert_eq!(p.status, "ok");
        let calls = runner.calls.lock().unwrap();
        assert!(calls[0].1.iter().any(|a| a == "-F"));
        assert!(calls[0]
            .1
            .iter()
            .any(|a| a.contains("horto-test-ssh-config")));
        drop(calls);
    }

    #[test]
    fn probe_ssh_access_prefers_stdout_when_stderr_empty() {
        let runner = ScriptedRunner::default();
        runner.push(
            "ssh",
            CommandOutput {
                status: 255,
                stdout: "Permission denied (publickey)".into(),
                stderr: String::new(),
            },
        );
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let p = probe_ssh_access(&runner, &opts).unwrap();
        assert_eq!(p.status, "auth failed");
    }

    #[test]
    fn read_local_api_token_rejects_non_hex() {
        let tmp = tempfile::TempDir::new().unwrap();
        with_xdg_config(&tmp, || {
            let path = api_token_config_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"HORTO_API_TOKEN=not-hex!!\n").unwrap();
            assert!(read_local_api_token().is_none());
            std::fs::write(&path, b"HORTO_API_TOKEN=\n").unwrap();
            assert!(read_local_api_token().is_none());
        });
    }

    #[test]
    fn probe_mcp_box_keeps_active_unit() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
        runner.push("ssh", ScriptedRunner::ok("active\n"));
        runner.push("ssh", ScriptedRunner::ok("active\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let report = probe_surfaces(&runner, &opts, false).unwrap();
        assert_eq!(report.mcp_box.unit, "active");
    }

    #[test]
    fn ssh_helpers_handle_bad_host_and_exec_err() {
        let runner = ScriptedRunner::default();
        let bad = RemoteOptions {
            host: String::new(),
            ..RemoteOptions::default()
        };
        assert_eq!(ssh_systemctl_active(&runner, &bad, "x.service"), "");
        assert_eq!(ssh_pgrep_mcp(&runner, &bad), "");

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::fail(1, "nope"));
        let ok_host = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        assert_eq!(ssh_systemctl_active(&runner2, &ok_host, "x.service"), "");
    }

    #[test]
    fn http_get_label_empty_and_garbage_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            drop(s);
        });
        assert_eq!(
            http_get_label("127.0.0.1", port, "/health", None),
            "unreachable"
        );

        let listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = listener2.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut s, _) = listener2.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"not-http-at-all");
        });
        assert_eq!(http_get_label("127.0.0.1", port2, "/health", None), "ok");
    }

    #[test]
    fn probe_api_status_when_health_ok_on_8787() {
        // Serialize :8787 binds across parallel surfaces tests.
        static PORT_8787: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = PORT_8787
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let listener = match TcpListener::bind("127.0.0.1:8787") {
            Ok(l) => l,
            Err(e) => {
                eprintln!("skip probe_api :8787 bind: {e}");
                return;
            }
        };
        thread::spawn(move || {
            for _ in 0..8 {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut buf = [0u8; 512];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\nok");
                }
            }
        });
        thread::sleep(std::time::Duration::from_millis(20));
        let runner = ScriptedRunner::default();
        let opts = RemoteOptions::default();
        let report = probe_surfaces(&runner, &opts, true).unwrap();
        assert_eq!(report.api.health, "ok");
        assert_eq!(report.api.status, "ok");
    }

    #[test]
    fn probe_one_surface_helpers_embedded() {
        let runner = ScriptedRunner::default();
        let opts = RemoteOptions::default();
        let ssh = probe_ssh_surface(&runner, &opts, true).unwrap();
        assert_eq!(ssh.status, "n/a");
        let cli = probe_cli_surface(&runner, &opts, true).unwrap();
        assert!(cli.current);
        // API/MCP hit localhost ports; only assert shape (no bind race with :8787 tests).
        let api = probe_api_surface(&runner, &opts, true).unwrap();
        assert!(api.url.starts_with("http://127.0.0.1:8787"));
        let (pc, bx) = probe_mcp_surface(&runner, &opts, true).unwrap();
        assert!(pc.http_url.starts_with("http://127.0.0.1:8790"));
        assert!(bx.http_url.starts_with("http://127.0.0.1:8790"));
    }

    #[test]
    fn find_mcp_docker_reports_image_when_inspect_ok() {
        let runner = ScriptedRunner::default();
        runner.push("docker", ScriptedRunner::ok("sha256:abc\n"));
        assert_eq!(
            find_mcp_docker(&runner).as_deref(),
            Some("horto-os-ui-mcp:local")
        );
    }

    #[test]
    fn find_mcp_docker_none_when_missing() {
        let runner = ScriptedRunner::default();
        runner.push("docker", ScriptedRunner::fail(1, "No such image"));
        assert!(find_mcp_docker(&runner).is_none());
    }

    #[test]
    fn probe_one_surface_helpers_remote() {
        let runner = ScriptedRunner::default();
        // probe_ssh_surface → BatchMode true
        runner.push("ssh", ScriptedRunner::ok(""));
        // probe_cli_surface → ssh again + remote version (first candidate)
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
        // probe_api_surface → ssh + status-api unit
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("active\n"));
        // probe_mcp_surface → ssh + mcp unit/pgrep + binary + docker
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("inactive\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box.example".into(),
            ..RemoteOptions::default()
        };
        let ssh = probe_ssh_surface(&runner, &opts, false).unwrap();
        assert_eq!(ssh.status, "ok");
        let cli = probe_cli_surface(&runner, &opts, false).unwrap();
        assert!(cli.current);
        let api = probe_api_surface(&runner, &opts, false).unwrap();
        assert!(api.url.contains("box.example"));
        let (pc, bx) = probe_mcp_surface(&runner, &opts, false).unwrap();
        assert!(pc.http_url.contains("box.example"));
        assert!(bx.http_url.contains("box.example"));
    }
}
