//! Remote CLI probe, arch detection, and box snapshot.

use super::super::arch::{box_arch_from_uname, BoxArch};
use super::super::process::{ProcessRunner, StdioMode};
use super::super::ssh::SshSession;
use super::options::{
    exec_remote_cli_captured, remote_cli_candidates, remote_install_bin, remote_progress,
    session_from, shell_quote, RemoteOptions,
};
use super::status::{parse_remote_json, parse_setup_status_text};
use crate::error::Result;
use crate::LONG_VERSION;

/// First line of `horto-os-ui --version` / long-version output.
#[must_use]
pub fn normalize_cli_version(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// Whether a remote `--version` line matches this PC's baked [`LONG_VERSION`].
#[must_use]
pub fn remote_cli_version_is_current(remote_version: &str) -> bool {
    let remote = normalize_cli_version(remote_version);
    if remote.is_empty() {
        return false;
    }
    remote == LONG_VERSION || remote == format!("horto-os-ui {LONG_VERSION}")
}

/// Probe `uname -m` on the remote host.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or the arch is unsupported.
pub fn remote_probe_arch(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Result<BoxArch> {
    let session = session_from(opts)?;
    remote_progress(&opts.host, "probe arch (SSH; may ask password)");
    let out = session.exec(runner, "uname -m", StdioMode::Capture)?;
    box_arch_from_uname(&out.stdout)
}

/// Result of probing CLI binaries already on the box (no SCP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCliProbe {
    /// Absolute path of the binary that answered `--version`, when any.
    pub path: Option<String>,
    /// Normalized version line from the box, when any.
    pub version: Option<String>,
    /// True when [`version`](Self::version) matches this PC's [`LONG_VERSION`].
    pub current: bool,
    /// Operator-facing box CLI label after a probe (never "not yet refreshed").
    pub status: RemoteBoxCliStatus,
}

/// How to show the box CLI after SSH was attempted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteBoxCliStatus {
    /// SSH worked; no `horto-os-ui --version` on install/agent paths.
    Missing,
    /// SSH authentication failed (password/key).
    AuthFailed,
    /// Host unreachable / DNS / connection refused / timeout.
    Unreachable,
    /// Version line from the box (current or stale).
    Found(String),
}

impl RemoteBoxCliStatus {
    /// Compact footer/overview label (`missing`, `auth failed`, version, …).
    #[must_use]
    pub const fn as_label(&self) -> &str {
        match self {
            Self::Missing => "missing",
            Self::AuthFailed => "auth failed",
            Self::Unreachable => "unreachable",
            Self::Found(v) => v.as_str(),
        }
    }
}

/// Classify OpenSSH failure text into auth vs unreachable (best-effort).
#[must_use]
pub fn classify_ssh_failure(detail: &str) -> RemoteBoxCliStatus {
    let d = detail.to_ascii_lowercase();
    if d.contains("permission denied")
        || d.contains("authentication failed")
        || d.contains("auth failed")
        || d.contains("too many authentication")
        || d.contains("no supported authentication")
        || d.contains("connection closed by remote host")
    {
        return RemoteBoxCliStatus::AuthFailed;
    }
    RemoteBoxCliStatus::Unreachable
}

pub fn ssh_command_detail(err: &crate::error::HortoError) -> Option<&str> {
    match err {
        crate::error::HortoError::CommandFailed { program, detail }
            if program == "ssh" || program == "scp" =>
        {
            Some(detail.as_str())
        }
        _ => None,
    }
}

pub const fn probe_from_status(status: RemoteBoxCliStatus) -> RemoteCliProbe {
    RemoteCliProbe {
        path: None,
        version: None,
        current: false,
        status,
    }
}

pub fn probe_found(path: String, version: String, current: bool) -> RemoteCliProbe {
    RemoteCliProbe {
        path: Some(path),
        version: Some(version.clone()),
        current,
        status: RemoteBoxCliStatus::Found(version),
    }
}

/// Probe install-dir then agent-dir CLI versions over SSH (never SCP).
///
/// # Errors
///
/// Returns [`crate::HortoError`] only for unexpected non-SSH failures. Auth /
/// unreachable / missing CLI are returned as [`Ok`] with [`RemoteCliProbe::status`].
pub fn probe_remote_cli(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
) -> Result<RemoteCliProbe> {
    remote_progress(&opts.host, "probe CLI version on box (SSH; no upload)");
    let mut last_version = None;
    let mut last_path = None;
    let mut saw_remote_cmd = false;
    for path in remote_cli_candidates(opts) {
        let cmd = format!("test -x {bin} && {bin} --version", bin = shell_quote(&path));
        match session.exec(runner, &cmd, StdioMode::Capture) {
            Ok(out) => {
                saw_remote_cmd = true;
                let ver = normalize_cli_version(&out.stdout);
                if ver.is_empty() {
                    continue;
                }
                if remote_cli_version_is_current(&ver) {
                    return Ok(probe_found(path, ver, true));
                }
                last_version = Some(ver);
                last_path = Some(path);
            }
            Err(e) => {
                if let Some(detail) = ssh_command_detail(&e) {
                    // Remote `test -x` failure is typically `exit 1:…` (SSH reached the box).
                    let low = detail.to_ascii_lowercase();
                    if low.contains("exit 1") || low.contains("exit 127") {
                        saw_remote_cmd = true;
                        continue;
                    }
                    return Ok(probe_from_status(classify_ssh_failure(detail)));
                }
                return Ok(probe_from_status(RemoteBoxCliStatus::Unreachable));
            }
        }
    }
    if let (Some(path), Some(ver)) = (last_path, last_version) {
        return Ok(probe_found(path, ver, false));
    }
    Ok(probe_from_status(if saw_remote_cmd {
        RemoteBoxCliStatus::Missing
    } else {
        RemoteBoxCliStatus::Unreachable
    }))
}

/// Setup status + doctor from the box without uploading (TUI `r`).
#[derive(Debug, Clone)]
pub struct RemoteBoxSnapshot {
    /// Whether box CLI long-version matches this PC.
    pub cli_current: bool,
    /// Box CLI version line when a binary answered `--version`.
    pub cli_version: Option<String>,
    /// Operator-facing box CLI status after the probe.
    pub cli_status: RemoteBoxCliStatus,
    /// Pipeline step rows from the box (only when [`cli_current`](Self::cli_current)).
    pub setup: Option<crate::ops::status::SetupStatusReport>,
    /// Doctor JSON from the box (only when current).
    pub doctor: Option<crate::ops::doctor::DoctorReport>,
}

/// Probe CLI version, then fetch setup status and doctor when current. Never SCP.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails in an unexpected way or remote
/// output cannot be parsed. Auth / unreachable / missing CLI are [`Ok`] with
/// [`RemoteBoxSnapshot::cli_status`] set accordingly.
pub fn remote_box_snapshot(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    full: bool,
) -> Result<RemoteBoxSnapshot> {
    let session = session_from(opts)?;
    let probe = probe_remote_cli(runner, &session, opts)?;
    if !probe.current {
        return Ok(RemoteBoxSnapshot {
            cli_current: false,
            cli_version: probe.version,
            cli_status: probe.status,
            setup: None,
            doctor: None,
        });
    }
    // `probe_remote_cli` always sets `path` when `current`; keep a safe fallback.
    let remote_bin = probe.path.unwrap_or_else(|| remote_install_bin(opts));

    let kind = if full { "--full" } else { "--minimal" };
    let json_log = exec_remote_cli_captured(
        runner,
        &session,
        &remote_bin,
        opts,
        &[
            "setup".into(),
            "status".into(),
            kind.into(),
            "--json".into(),
        ],
    );
    let setup = if let Some(report) = json_log.ok().and_then(|log| parse_remote_json(&log).ok()) {
        report
    } else {
        let text_log = exec_remote_cli_captured(
            runner,
            &session,
            &remote_bin,
            opts,
            &["setup".into(), "status".into(), kind.into()],
        )?;
        parse_setup_status_text(&text_log)?
    };

    let doctor_log =
        exec_remote_cli_captured(runner, &session, &remote_bin, opts, &["doctor".into()])?;
    let doctor = parse_remote_json(&doctor_log)?;
    Ok(RemoteBoxSnapshot {
        cli_current: true,
        cli_version: probe.version.clone(),
        cli_status: probe.status,
        setup: Some(setup),
        doctor: Some(doctor),
    })
}
