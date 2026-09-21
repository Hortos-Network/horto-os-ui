//! High-level remote apply: probe arch, drop CLI agent, run setup, install payload.

use super::arch::{box_arch_from_uname, BoxArch};
use super::bins::{ensure_local_bins, LocalBins};
use super::ecosystem::{
    remote_enable_ecosystem_script, API_TOKEN_DROP_BASENAME, DEFAULT_INSTALL_DIR as ECO_INSTALL_DIR,
};
use super::host::parse_host_spec;
use super::process::{ProcessRunner, StdioMode};
use super::ssh::{SshEnv, SshSession};
use super::transfer::transfer_files;
use crate::error::Result;
use crate::{LONG_VERSION, VERSION};
use std::path::{Path, PathBuf};

/// Default GitHub repo that publishes box Release tar.gz assets.
pub const DEFAULT_GITHUB_REPO: &str = "Hortos-Network/horto-os-ui";

/// Remote directory for the temporary CLI apply agent.
pub const DEFAULT_REMOTE_AGENT_DIR: &str = "/tmp/horto-os-ui-remote";

/// Permanent install directory for CLI + TUI + status-api + MCP on the box.
pub const DEFAULT_INSTALL_DIR: &str = ECO_INSTALL_DIR;

/// Options for a remote OpenSSH session and binary source.
#[derive(Debug, Clone)]
pub struct RemoteOptions {
    /// OpenSSH Host alias or `user@host`.
    pub host: String,
    /// When true, run `ssh-copy-id` once before apply. Default false.
    pub install_ssh_key: bool,
    /// Local directory with the four box binaries (skips GitHub download).
    pub bin_dir: Option<PathBuf>,
    /// Workspace / Release version (`0.1.0`, without `v`). Used in asset filenames.
    pub version: String,
    /// GitHub Release tag to download from (`v0.1.0` or tip `dev-preview`).
    pub release_tag: String,
    /// `owner/repo` for Release downloads.
    pub github_repo: String,
    /// Force `SSH_ASKPASS` (Desktop / no TTY).
    pub force_askpass: bool,
    /// Cache root for downloaded Release assets.
    pub cache_root: PathBuf,
    /// Remote temp dir for the apply agent.
    pub remote_agent_dir: String,
    /// Remote install dir for permanent binaries.
    pub install_dir: String,
    /// Optional OpenSSH config file (`ssh -F`). Used by tests and custom layouts.
    pub ssh_config_file: Option<PathBuf>,
}

impl Default for RemoteOptions {
    fn default() -> Self {
        let version = VERSION.to_owned();
        let release_tag = super::bins::default_release_tag(&version);
        Self {
            host: String::new(),
            install_ssh_key: false,
            bin_dir: None,
            version,
            release_tag,
            github_repo: DEFAULT_GITHUB_REPO.to_owned(),
            force_askpass: false,
            cache_root: super::bins::default_cache_root(),
            remote_agent_dir: DEFAULT_REMOTE_AGENT_DIR.to_owned(),
            install_dir: DEFAULT_INSTALL_DIR.to_owned(),
            ssh_config_file: None,
        }
    }
}

/// CLI arguments to run on the box via the uploaded agent (without the binary name).
#[derive(Debug, Clone)]
pub struct RemoteRunRequest {
    /// Session and binary options.
    pub options: RemoteOptions,
    /// Arguments after the remote `horto-os-ui` binary (e.g. `["setup", "run", "--full"]` or with `--apply`).
    pub cli_args: Vec<String>,
    /// Prefix with `sudo -n` / `sudo` when true (apply mode).
    pub use_sudo: bool,
    /// After a successful command, install CLI+TUI+status-api+MCP and enable both units.
    pub install_payload_on_success: bool,
    /// After success (and payload install), offer an interactive box reboot (TTY only).
    pub offer_reboot_on_success: bool,
    /// Capture remote stdout/stderr instead of inheriting the local TTY.
    ///
    /// Use for read-only status/doctor so callers can parse JSON. Keep false for
    /// apply paths that need interactive sudo / password prompts.
    pub capture_output: bool,
}

pub(crate) fn session_from(opts: &RemoteOptions) -> Result<SshSession> {
    Ok(SshSession {
        host: parse_host_spec(&opts.host)?,
        env: SshEnv {
            force_askpass: opts.force_askpass,
        },
        config_file: opts.ssh_config_file.clone(),
    })
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_owned();
    }
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | ':' | '+'))
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Format an operator progress banner for remote SSH/SCP/key steps.
#[must_use]
pub fn remote_progress_message(host: &str, detail: &str) -> String {
    format!("[horto remote] PC → box '{host}': {detail}")
}

/// Detail line for a remote CLI run banner.
#[must_use]
pub fn remote_run_banner_detail(remote_cmd: &str, use_sudo: bool) -> String {
    if use_sudo {
        format!("run `{remote_cmd}` (SSH + sudo; may ask password)")
    } else {
        format!("run `{remote_cmd}` (SSH; may ask password)")
    }
}

fn remote_progress(host: &str, detail: &str) {
    tracing::info!("{}", remote_progress_message(host, detail));
}

/// Log the remote doctor JSON header (stderr via tracing).
pub fn remote_doctor_report_banner() {
    tracing::info!("[horto remote] doctor report from box (JSON):");
}

fn remote_agent_bin(opts: &RemoteOptions) -> String {
    format!(
        "{}/horto-os-ui",
        opts.remote_agent_dir.trim_end_matches('/')
    )
}

fn remote_install_bin(opts: &RemoteOptions) -> String {
    format!("{}/horto-os-ui", opts.install_dir.trim_end_matches('/'))
}

fn remote_cli_candidates(opts: &RemoteOptions) -> [String; 2] {
    [remote_install_bin(opts), remote_agent_bin(opts)]
}

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

fn build_remote_command_at(bin: &str, cli_args: &[String], use_sudo: bool) -> String {
    let mut parts = Vec::new();
    if use_sudo {
        parts.push("sudo".to_owned());
    }
    parts.push(shell_quote(bin));
    for a in cli_args {
        parts.push(shell_quote(a));
    }
    parts.join(" ")
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
    pub fn as_label(&self) -> &str {
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

fn ssh_command_detail(err: &crate::error::HortoError) -> Option<&str> {
    match err {
        crate::error::HortoError::CommandFailed { program, detail }
            if program == "ssh" || program == "scp" =>
        {
            Some(detail.as_str())
        }
        _ => None,
    }
}

fn probe_from_status(status: RemoteBoxCliStatus) -> RemoteCliProbe {
    RemoteCliProbe {
        path: None,
        version: None,
        current: false,
        status,
    }
}

fn probe_found(path: String, version: String, current: bool) -> RemoteCliProbe {
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

fn upload_remote_cli(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
    bins: &LocalBins,
) -> Result<String> {
    session.exec(
        runner,
        &format!("mkdir -p {}", shell_quote(&opts.remote_agent_dir)),
        StdioMode::Capture,
    )?;
    let remote_bin = remote_agent_bin(opts);
    remote_progress(&opts.host, "upload CLI agent (SCP; may ask password)");
    session.scp_to(runner, &bins.cli, &remote_bin)?;
    session.exec(
        runner,
        &format!("chmod +x {}", shell_quote(&remote_bin)),
        StdioMode::Capture,
    )?;
    Ok(remote_bin)
}

/// Opt-in: install the tip pubkey on the box (`ssh-copy-id`).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH or `ssh-copy-id` fails.
pub fn remote_ensure_ssh_key(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Result<()> {
    let session = session_from(opts)?;
    session.install_ssh_key(runner)
}

/// Issue `sudo reboot` on the box (CLI: password on the terminal via Inherit).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails before reboot starts.
pub fn remote_reboot(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Result<()> {
    let session = session_from(opts)?;
    finish_remote_reboot(runner, &session, "y", None)
}

/// Issue `sudo reboot` using a sudo password already collected by the UI.
///
/// Feeds `sudo -S` over captured SSH. Used by the TUI (no cooked TTY, no askpass).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails before reboot starts.
pub fn remote_reboot_with_sudo_password(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    sudo_password: &str,
) -> Result<()> {
    let session = session_from(opts)?;
    finish_remote_reboot(runner, &session, "y", Some(sudo_password))
}

/// Upload the PC CLI to the box agent dir (s0 / explicit sync).
///
/// # Errors
///
/// Returns [`crate::HortoError`] on SSH/SCP/local-bin failures.
pub fn remote_upload_cli(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
) -> Result<RemoteCliProbe> {
    let session = session_from(opts)?;
    remote_progress(&opts.host, "probe arch (SSH; may ask password)");
    let out = session.exec(runner, "uname -m", StdioMode::Capture)?;
    let arch = box_arch_from_uname(&out.stdout)?;
    let bins = ensure_local_bins(
        runner,
        &opts.release_tag,
        &opts.version,
        &opts.github_repo,
        arch,
        opts.bin_dir.as_deref(),
        &opts.cache_root,
    )?;
    upload_remote_cli(runner, &session, opts, &bins)?;
    maybe_install_key(runner, &session, opts)?;
    probe_remote_cli(runner, &session, opts)
}

/// Ensure a current CLI on the box: reuse when version matches, otherwise SCP once.
fn ensure_remote_cli(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
    bins: &LocalBins,
) -> Result<String> {
    let probe = probe_remote_cli(runner, session, opts)?;
    if probe.current {
        if let Some(path) = probe.path {
            remote_progress(&opts.host, "CLI on box already current; skip upload");
            return Ok(path);
        }
    }
    upload_remote_cli(runner, session, opts, bins)
}

fn maybe_install_key(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
) -> Result<()> {
    if opts.install_ssh_key {
        session.install_ssh_key(runner)?;
    }
    Ok(())
}

/// Prepare OpenSSH session and local tip bins; SCP CLI only when box is stale/missing.
fn prepare_remote_agent(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
) -> Result<(SshSession, LocalBins, String)> {
    let session = session_from(opts)?;
    remote_progress(&opts.host, "probe arch (SSH; may ask password)");
    let out = session.exec(runner, "uname -m", StdioMode::Capture)?;
    let arch = box_arch_from_uname(&out.stdout)?;
    let bins = ensure_local_bins(
        runner,
        &opts.release_tag,
        &opts.version,
        &opts.github_repo,
        arch,
        opts.bin_dir.as_deref(),
        &opts.cache_root,
    )?;
    let remote_bin = ensure_remote_cli(runner, &session, opts, &bins)?;
    maybe_install_key(runner, &session, opts)?;
    Ok((session, bins, remote_bin))
}

fn merge_command_log(out: &super::process::CommandOutput, remote_cmd: &str) -> String {
    let mut log = out.stdout.clone();
    if !out.stderr.trim().is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&out.stderr);
    }
    if log.trim().is_empty() {
        format!("remote command finished: {remote_cmd}")
    } else {
        log
    }
}

/// Run one remote CLI argv list on an already-resolved box binary (Capture).
fn exec_remote_cli_captured(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    remote_bin: &str,
    opts: &RemoteOptions,
    cli_args: &[String],
) -> Result<String> {
    let remote_cmd = build_remote_command_at(remote_bin, cli_args, false);
    remote_progress(&opts.host, &remote_run_banner_detail(&remote_cmd, false));
    let out = session.exec(runner, &remote_cmd, StdioMode::Capture)?;
    Ok(merge_command_log(&out, &remote_cmd))
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
    opts: RemoteOptions,
    full: bool,
) -> Result<RemoteBoxSnapshot> {
    let session = session_from(&opts)?;
    let probe = probe_remote_cli(runner, &session, &opts)?;
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
    let remote_bin = probe.path.unwrap_or_else(|| remote_install_bin(&opts));

    let kind = if full { "--full" } else { "--minimal" };
    let json_log = exec_remote_cli_captured(
        runner,
        &session,
        &remote_bin,
        &opts,
        &[
            "setup".into(),
            "status".into(),
            kind.into(),
            "--json".into(),
        ],
    );
    let setup = match json_log.ok().and_then(|log| parse_remote_json(&log).ok()) {
        Some(report) => report,
        None => {
            let text_log = exec_remote_cli_captured(
                runner,
                &session,
                &remote_bin,
                &opts,
                &["setup".into(), "status".into(), kind.into()],
            )?;
            parse_setup_status_text(&text_log)?
        }
    };

    let doctor_log =
        exec_remote_cli_captured(runner, &session, &remote_bin, &opts, &["doctor".into()])?;
    let doctor = parse_remote_json(&doctor_log)?;
    Ok(RemoteBoxSnapshot {
        cli_current: true,
        cli_version: probe.version.clone(),
        cli_status: probe.status,
        setup: Some(setup),
        doctor: Some(doctor),
    })
}

/// Result of a remote CLI run (log text plus optional captured status-api bearer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRunOutcome {
    /// Merged remote stdout/stderr (or a short finished marker when Inherit was empty).
    pub log: String,
    /// Hex bearer from payload install, when captured from the box drop file.
    pub api_token: Option<String>,
}

/// Upload the CLI agent (if needed), optionally install a public key, run a remote CLI command.
///
/// # Errors
///
/// Returns [`crate::HortoError`] on SSH/SCP/agent failures.
pub fn remote_run_cli(
    runner: &dyn ProcessRunner,
    req: &RemoteRunRequest,
) -> Result<RemoteRunOutcome> {
    let opts = &req.options;
    let (session, bins, remote_bin) = prepare_remote_agent(runner, opts)?;

    let remote_cmd = build_remote_command_at(&remote_bin, &req.cli_args, req.use_sudo);
    remote_progress(
        &opts.host,
        &remote_run_banner_detail(&remote_cmd, req.use_sudo),
    );
    let stdio = if req.capture_output {
        StdioMode::Capture
    } else {
        // Inherit so SSH/sudo password prompts work on a TTY (CLI/TUI apply).
        StdioMode::Inherit
    };
    let out = session.exec(runner, &remote_cmd, stdio)?;
    let log = merge_command_log(&out, &remote_cmd);

    let mut api_token = None;
    if req.install_payload_on_success {
        api_token = remote_install_payload(runner, opts, &bins)?;
    }
    if req.offer_reboot_on_success {
        offer_remote_reboot(runner, &session)?;
    }
    Ok(RemoteRunOutcome { log, api_token })
}

/// Whether a reboot prompt answer means reboot now.
#[must_use]
pub fn wants_reboot_now(raw: &str) -> bool {
    matches!(raw.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Prompt on a local TTY, then `sudo reboot` on the box. Non-TTY prints a reminder.
fn offer_remote_reboot(runner: &dyn ProcessRunner, session: &SshSession) -> Result<()> {
    use std::io::{self, IsTerminal, Write};

    if !io::stdin().is_terminal() {
        eprintln!("Reboot recommended for hostname/network changes. On the box: sudo reboot");
        return Ok(());
    }
    eprint!("Reboot the box now to apply hostname/network changes? [y/N]: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| crate::error::HortoError::msg(format!("read reboot prompt: {e}")))?;
    finish_remote_reboot(runner, session, &line, None)
}

fn finish_remote_reboot(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    answer: &str,
    sudo_password: Option<&str>,
) -> Result<()> {
    if !wants_reboot_now(answer) {
        eprintln!("Skipping reboot. Reboot the box later when convenient.");
        return Ok(());
    }
    remote_progress(
        &session.host.raw,
        "reboot box (SSH + sudo; may ask password)",
    );
    eprintln!("Rebooting...");
    let used_stdin = sudo_password.is_some();
    let result = if let Some(pass) = sudo_password {
        let mut feed = String::with_capacity(pass.len() + 1);
        feed.push_str(pass);
        feed.push('\n');
        let out = session.exec_stdin_reboot(runner, "sudo -S reboot", feed.as_bytes());
        feed.clear();
        out
    } else {
        session.exec(runner, "sudo reboot", StdioMode::Inherit)
    };
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            if used_stdin && !ssh_drop_after_reboot(&e) {
                return Err(e);
            }
            // Host drop mid-session is expected once reboot starts.
            eprintln!("reboot issued (SSH session closed is expected): {e}");
            Ok(())
        }
    }
}

fn ssh_drop_after_reboot(err: &crate::error::HortoError) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("connection closed")
        || s.contains("connection reset")
        || s.contains("broken pipe")
        || s.contains("exit 255")
}

/// Convenience: remote `setup run` with optional payload install.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the remote setup fails.
pub fn remote_setup_run(
    runner: &dyn ProcessRunner,
    opts: RemoteOptions,
    apply: bool,
    full: bool,
    skip_piper: bool,
    install_payload: bool,
) -> Result<RemoteRunOutcome> {
    let mut cli_args = Vec::new();
    if apply {
        cli_args.push("--apply".into());
    }
    if skip_piper {
        cli_args.push("--skip-piper".into());
    }
    cli_args.push("setup".into());
    cli_args.push("run".into());
    if full {
        cli_args.push("--full".into());
    } else {
        cli_args.push("--minimal".into());
    }
    remote_run_cli(
        runner,
        &RemoteRunRequest {
            options: opts,
            cli_args,
            use_sudo: apply,
            install_payload_on_success: install_payload && apply,
            offer_reboot_on_success: apply,
            capture_output: false,
        },
    )
}

/// Parse the first JSON object/array from remote captured output.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when no JSON is found or deserialization fails.
pub fn parse_remote_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    let start = raw.find('{').or_else(|| raw.find('[')).ok_or_else(|| {
        crate::error::HortoError::msg("remote output had no JSON object or array")
    })?;
    serde_json::from_str(raw[start..].trim())
        .map_err(|e| crate::error::HortoError::msg(format!("parse remote JSON: {e}")))
}

/// Parse human `setup status` lines from older box agents (no `--json`).
fn parse_setup_status_text(raw: &str) -> Result<crate::ops::status::SetupStatusReport> {
    use crate::ops::status::{SetupStatusReport, StepStatusRow};
    let mut kind = "full".to_owned();
    let mut steps = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Setup kind:") {
            kind = rest.trim().to_owned();
            continue;
        }
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((status_part, after_status)) = rest.split_once(']') else {
            continue;
        };
        let status = status_part.trim().to_owned();
        let after_status = after_status.trim();
        let Some((id, after_id)) = after_status.split_once(" - ") else {
            continue;
        };
        let (title, ver_flags) = match after_id.rfind(" (v") {
            Some(i) => (&after_id[..i], &after_id[i..]),
            None => (after_id, ""),
        };
        let step_version = ver_flags
            .trim_start_matches(" (v")
            .split(')')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        steps.push(StepStatusRow {
            id: id.trim().to_owned(),
            title: title.trim().to_owned(),
            status,
            step_version,
            destructive: ver_flags.contains("[destructive]"),
            needs_reboot_after: ver_flags.contains("[reboot]"),
        });
    }
    if steps.is_empty() {
        return Err(crate::error::HortoError::msg(
            "remote setup status produced no step lines",
        ));
    }
    Ok(SetupStatusReport { kind, steps })
}

/// Fetch setup step status from the box (`setup status`, JSON when supported).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or status output cannot be parsed.
pub fn remote_setup_status(
    runner: &dyn ProcessRunner,
    opts: RemoteOptions,
    full: bool,
) -> Result<crate::ops::status::SetupStatusReport> {
    let snap = remote_box_snapshot(runner, opts, full)?;
    snap.setup.ok_or_else(|| {
        crate::error::HortoError::msg(
            "box CLI missing or outdated; sync CLI (s0) before setup status",
        )
    })
}

/// Fetch doctor JSON from the box.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or the remote JSON cannot be parsed.
pub fn remote_doctor(
    runner: &dyn ProcessRunner,
    opts: RemoteOptions,
) -> Result<crate::ops::doctor::DoctorReport> {
    let snap = remote_box_snapshot(runner, opts, true)?;
    snap.doctor.ok_or_else(|| {
        crate::error::HortoError::msg("box CLI missing or outdated; sync CLI (s0) before doctor")
    })
}

/// Parse drop-file contents: `HORTO_API_TOKEN=<hex>` or bare hex.
#[must_use]
pub fn parse_api_token_drop(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let hex = line.strip_prefix("HORTO_API_TOKEN=").unwrap_or(line).trim();
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_owned())
}

pub(crate) fn api_token_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("horto-os-ui").join("api_token");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("horto-os-ui")
        .join("api_token")
}

/// Write hex bearer to the local config path with mode `0o600`.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the directory or file cannot be written.
pub fn write_api_token_file(token: &str) -> Result<PathBuf> {
    use std::fs;
    use std::io::Write;

    let path = api_token_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            crate::error::HortoError::msg(format!("create {}: {e}", parent.display()))
        })?;
    }
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| {
                    crate::error::HortoError::msg(format!("write {}: {e}", path.display()))
                })?;
            writeln!(f, "{token}").map_err(|e| {
                crate::error::HortoError::msg(format!("write {}: {e}", path.display()))
            })?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&path, format!("{token}\n")).map_err(|e| {
                crate::error::HortoError::msg(format!("write {}: {e}", path.display()))
            })?;
        }
    }
    Ok(path)
}

/// Finish a save-token prompt given the raw answer (test seam).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the file cannot be written.
pub fn finish_save_api_token(token: &str, answer: &str) -> Result<bool> {
    if !wants_reboot_now(answer) {
        return Ok(false);
    }
    let path = write_api_token_file(token)?;
    tracing::info!(
        path = %path.display(),
        "saved status-api bearer (hex only; not logged)"
    );
    Ok(true)
}

/// Propose saving the status-api bearer to `~/.config/horto-os-ui/api_token`.
///
/// TTY: `[y/N]` prompt. Non-TTY: skip write and log the hex once for paste.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when stdin cannot be read or the file write fails.
pub fn offer_save_api_token(token: &str) -> Result<bool> {
    use std::io::IsTerminal;
    offer_save_api_token_with(token, std::io::stdin().is_terminal(), None)
}

/// Testable core of [`offer_save_api_token`] (`canned_answer` skips stdin when `Some`).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when stdin cannot be read or the file write fails.
pub(crate) fn offer_save_api_token_with(
    token: &str,
    is_tty: bool,
    canned_answer: Option<&str>,
) -> Result<bool> {
    use std::io::{self, Write};

    if !is_tty {
        tracing::info!(
            token,
            "status-api bearer (non-TTY; paste into Desktop Connection or save manually)"
        );
        return Ok(false);
    }
    eprint!("Save status-api bearer to ~/.config/horto-os-ui/api_token? [y/N]: ");
    let _ = io::stderr().flush();
    let line = if let Some(answer) = canned_answer {
        answer.to_owned()
    } else {
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|e| crate::error::HortoError::msg(format!("read token save prompt: {e}")))?;
        line
    };
    finish_save_api_token(token, &line)
}

/// Copy CLI + TUI + status-api + MCP into `install_dir` and enable both systemd units.
///
/// Returns the captured hex bearer when the drop file can be read.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when transfer or remote enable fails.
pub fn remote_install_payload(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    bins: &LocalBins,
) -> Result<Option<String>> {
    let session = session_from(opts)?;
    let staging = format!("{}/payload", opts.remote_agent_dir.trim_end_matches('/'));
    remote_progress(
        &opts.host,
        "install box payload (SCP/SSH; may ask password)",
    );
    session.exec(
        runner,
        &format!("mkdir -p {}", shell_quote(&staging)),
        StdioMode::Capture,
    )?;

    let locals: [&Path; 4] = [&bins.cli, &bins.tui, &bins.status_api, &bins.mcp];
    transfer_files(runner, &session, &locals, &staging)?;

    let install = opts.install_dir.trim_end_matches('/');
    // One Inherit SSH session so sudo caches the credential across mkdir/tee/install/enable
    // (separate ssh invocations each re-prompt). Banner names the whole privileged block.
    remote_progress(
        &opts.host,
        "enable status-api + MCP on box: api.env token, systemd units, install bins (SSH + sudo; may ask password)",
    );
    let enable = remote_enable_ecosystem_script(&staging, install);
    session.exec(runner, &enable, StdioMode::Inherit)?;

    let drop_path = format!("$HOME/{API_TOKEN_DROP_BASENAME}");
    let cat_out = session.exec(
        runner,
        &format!("cat {drop_path} 2>/dev/null || true"),
        StdioMode::Capture,
    )?;
    let api_token = parse_api_token_drop(&cat_out.stdout);
    let _ = session.exec(runner, &format!("rm -f {drop_path}"), StdioMode::Capture);

    Ok(api_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::process::{CommandOutput, ScriptedRunner};
    use std::fs;
    use tempfile::TempDir;

    /// Env vars for config path are process-global; serialize tests that mutate them.
    use crate::remote::ENV_LOCK as CONFIG_ENV_LOCK;

    fn bin_dir_with_stubs() -> TempDir {
        let tmp = TempDir::new().unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(tmp.path().join(name), b"#!/bin/true\n").unwrap();
        }
        tmp
    }

    #[test]
    fn shell_quote_safe_and_unsafe() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("setup"), "setup");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn remote_progress_message_labels_host_and_detail() {
        assert_eq!(
            remote_progress_message("horto", "probe arch (SSH; may ask password)"),
            "[horto remote] PC → box 'horto': probe arch (SSH; may ask password)"
        );
    }

    #[test]
    fn parse_setup_status_text_reads_cli_lines() {
        let raw = "\
Setup kind: full
  [  done] s1 - Install base packages (v1)
  [pending] s5 - Apply staged configs to /etc (v2) [destructive] [reboot]
";
        let report = parse_setup_status_text(raw).unwrap();
        assert_eq!(report.kind, "full");
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[0].id, "s1");
        assert_eq!(report.steps[0].status, "done");
        assert_eq!(report.steps[1].id, "s5");
        assert!(report.steps[1].destructive);
        assert!(report.steps[1].needs_reboot_after);
    }

    #[test]
    fn parse_setup_status_text_skips_noise_and_allows_title_without_version() {
        let raw = "\
noise before
Setup kind: minimal
[broken
[x] no dash here
  [  done] s2 - Title only
";
        let report = parse_setup_status_text(raw).unwrap();
        assert_eq!(report.kind, "minimal");
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].id, "s2");
        assert_eq!(report.steps[0].title, "Title only");
        assert_eq!(report.steps[0].step_version, 0);
    }

    #[test]
    fn parse_setup_status_text_errors_without_steps() {
        let err = parse_setup_status_text("Setup kind: full\njust noise\n").unwrap_err();
        assert!(err.to_string().contains("no step lines"));
    }

    #[test]
    fn parse_remote_json_skips_leading_noise() {
        let raw = "ssh warn\n{\"is_root\":false,\"has_sudo\":true,\"docker_present\":true,\"active_setup_dir\":true,\"full_env\":true,\"minimal_env\":false,\"docker_dir\":true,\"backup_dir\":true,\"notes\":[]}\n";
        let doc: crate::ops::doctor::DoctorReport = parse_remote_json(raw).unwrap();
        assert!(doc.has_sudo);
        assert!(doc.docker_present);
    }

    #[test]
    fn remote_run_banner_detail_marks_sudo() {
        assert!(remote_run_banner_detail("horto-os-ui doctor", false).contains("SSH;"));
        assert!(!remote_run_banner_detail("horto-os-ui doctor", false).contains("sudo"));
        assert!(remote_run_banner_detail("sudo horto-os-ui setup", true).contains("SSH + sudo"));
    }

    #[test]
    fn remote_progress_and_doctor_banner_emit() {
        remote_progress("box", "unit-test detail");
        remote_doctor_report_banner();
    }

    fn push_cli_probes_missing(runner: &ScriptedRunner) {
        // install_dir then agent_dir: test -x fails → ssh non-zero
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
    }

    fn push_cli_probe_current(runner: &ScriptedRunner) {
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
    }

    #[test]
    fn normalize_and_match_cli_version() {
        assert_eq!(
            normalize_cli_version(&format!("  {LONG_VERSION} \n")),
            LONG_VERSION
        );
        assert!(remote_cli_version_is_current(LONG_VERSION));
        assert!(remote_cli_version_is_current(&format!(
            "horto-os-ui {LONG_VERSION}"
        )));
        assert!(!remote_cli_version_is_current("0.0.0 (deadbeef)"));
        assert!(!remote_cli_version_is_current(""));
    }

    #[test]
    fn classify_ssh_failure_auth_vs_unreachable() {
        assert_eq!(
            classify_ssh_failure("exit 255: Permission denied (publickey,password)"),
            RemoteBoxCliStatus::AuthFailed
        );
        assert_eq!(
            classify_ssh_failure("exit 255: Connection refused"),
            RemoteBoxCliStatus::Unreachable
        );
        assert_eq!(
            classify_ssh_failure("exit 255: Could not resolve hostname"),
            RemoteBoxCliStatus::Unreachable
        );
    }

    #[test]
    fn remote_box_cli_status_as_label_covers_all_variants() {
        assert_eq!(RemoteBoxCliStatus::Missing.as_label(), "missing");
        assert_eq!(RemoteBoxCliStatus::AuthFailed.as_label(), "auth failed");
        assert_eq!(RemoteBoxCliStatus::Unreachable.as_label(), "unreachable");
        assert_eq!(
            RemoteBoxCliStatus::Found("0.1.0 (abc)".into()).as_label(),
            "0.1.0 (abc)"
        );
    }

    #[test]
    fn ssh_command_detail_filters_non_ssh_errors() {
        assert!(ssh_command_detail(&crate::error::HortoError::msg("nope")).is_none());
        assert_eq!(
            ssh_command_detail(&crate::error::HortoError::command("ssh", "exit 255: x")),
            Some("exit 255: x")
        );
        assert_eq!(
            ssh_command_detail(&crate::error::HortoError::command("scp", "denied")),
            Some("denied")
        );
        assert!(ssh_command_detail(&crate::error::HortoError::command("tar", "x")).is_none());
    }

    #[test]
    fn probe_remote_cli_reports_unreachable_on_connection_refused() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection refused"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::Unreachable);
    }

    #[test]
    fn probe_remote_cli_reports_auth_failed() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::AuthFailed);
        assert!(!probe.current);
    }

    #[test]
    fn probe_remote_cli_reports_missing_when_binaries_absent() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::Missing);
    }

    #[test]
    fn remote_box_snapshot_no_scp_when_cli_current() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":["ok"]}"#,
            ),
        );

        let snap = remote_box_snapshot(
            &runner,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert!(snap.cli_current);
        assert_eq!(snap.setup.as_ref().unwrap().steps[0].id, "s1");
        assert!(snap.doctor.as_ref().unwrap().has_sudo);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0,
            "status refresh must never SCP when CLI is current"
        );
    }

    #[test]
    fn remote_box_snapshot_stale_skips_status_without_scp() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);

        let snap = remote_box_snapshot(
            &runner,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert!(!snap.cli_current);
        assert!(snap.setup.is_none());
        assert!(snap.doctor.is_none());
        assert_eq!(snap.cli_status, RemoteBoxCliStatus::Missing);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_box_snapshot_falls_back_to_text_status() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("not json\n"));
        runner.push(
            "ssh",
            ScriptedRunner::ok("Setup kind: full\n  [  done] s1 - Install base packages (v1)\n"),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":false,"active_setup_dir":false,"full_env":false,"minimal_env":true,"docker_dir":false,"backup_dir":false,"notes":[]}"#,
            ),
        );

        let snap = remote_box_snapshot(
            &runner,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert_eq!(snap.setup.as_ref().unwrap().steps[0].id, "s1");
        assert!(!snap.doctor.as_ref().unwrap().docker_present);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_upload_cli_scps_once() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));

        let probe = remote_upload_cli(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                bin_dir: Some(stubs.path().to_path_buf()),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert!(probe.current);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            1
        );
    }

    #[test]
    fn probe_remote_cli_skips_empty_version_then_accepts_agent() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("\n"));
        push_cli_probe_current(&runner);
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert!(probe.current);
        assert!(probe.path.unwrap().contains("horto-os-ui"));
    }

    #[test]
    fn probe_remote_cli_keeps_stale_install_when_agent_missing() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("0.0.0 (deadbeef)\n"));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert!(!probe.current);
        assert_eq!(probe.version.as_deref(), Some("0.0.0 (deadbeef)"));
        assert!(probe.path.is_some());
    }

    #[test]
    fn remote_run_skips_scp_when_box_cli_already_current() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("ok\n"));

        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: false,
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("ok"));
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_run_capture_output_merges_stderr() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            CommandOutput {
                status: 0,
                stdout: "out-line\n".into(),
                stderr: "warn-line\n".into(),
            },
        );

        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: true,
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("out-line"));
        assert!(log.contains("warn-line"));
    }

    #[test]
    fn remote_setup_status_and_doctor_require_current_cli() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);
        let err = remote_setup_status(
            &runner,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("sync CLI"));

        let runner2 = ScriptedRunner::default();
        push_cli_probes_missing(&runner2);
        let err = remote_doctor(
            &runner2,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("sync CLI"));
    }

    #[test]
    fn remote_setup_status_and_doctor_return_reports_when_current() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":[]}"#,
            ),
        );
        let status = remote_setup_status(
            &runner,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert_eq!(status.steps[0].id, "s1");

        let runner2 = ScriptedRunner::default();
        push_cli_probe_current(&runner2);
        runner2.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner2.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":[]}"#,
            ),
        );
        let doc = remote_doctor(
            &runner2,
            RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert!(doc.has_sudo);
    }

    #[test]
    fn build_remote_command_shapes() {
        let cmd = build_remote_command_at(
            "/tmp/agent/horto-os-ui",
            &[
                "--apply".into(),
                "setup".into(),
                "run".into(),
                "--full".into(),
            ],
            false,
        );
        assert_eq!(cmd, "/tmp/agent/horto-os-ui --apply setup run --full");
        let cmd_sudo = build_remote_command_at("/tmp/agent/horto-os-ui", &["doctor".into()], true);
        assert!(cmd_sudo.starts_with("sudo "));
    }

    #[test]
    fn remote_run_does_not_copy_id_by_default() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        // uname
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        // mkdir agent
        runner.push("ssh", ScriptedRunner::ok(""));
        // scp agent
        runner.push("scp", ScriptedRunner::ok(""));
        // chmod
        runner.push("ssh", ScriptedRunner::ok(""));
        // remote cli
        runner.push("ssh", ScriptedRunner::ok("ok\n"));

        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    install_ssh_key: false,
                    ..RemoteOptions::default()
                },
                cli_args: vec!["setup".into(), "status".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: false,
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("ok"));
        let programs: Vec<_> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _, _, _)| p.clone())
            .collect();
        assert!(!programs.iter().any(|p| p == "ssh-copy-id"));
    }

    #[test]
    fn remote_run_install_key_when_opt_in() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let stubs = bin_dir_with_stubs();
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::ok("aarch64\n"));
            push_cli_probes_missing(&runner);
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: not authorized yet.
            runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
            runner.push("ssh-copy-id", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok("done\n"));

            remote_run_cli(
                &runner,
                &RemoteRunRequest {
                    options: RemoteOptions {
                        host: "box".into(),
                        bin_dir: Some(stubs.path().to_path_buf()),
                        install_ssh_key: true,
                        ..RemoteOptions::default()
                    },
                    cli_args: vec!["doctor".into()],
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                },
            )
            .unwrap();
            assert!(runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, _, _)| p == "ssh-copy-id"));
        });
    }

    #[test]
    fn remote_run_skips_ssh_copy_id_when_key_works() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let stubs = bin_dir_with_stubs();
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
            push_cli_probes_missing(&runner);
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: already authorized.
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok("doctor ok\n"));

            remote_run_cli(
                &runner,
                &RemoteRunRequest {
                    options: RemoteOptions {
                        host: "box".into(),
                        bin_dir: Some(stubs.path().to_path_buf()),
                        install_ssh_key: true,
                        ..RemoteOptions::default()
                    },
                    cli_args: vec!["doctor".into()],
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                },
            )
            .unwrap();
            assert!(!runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, _, _)| p == "ssh-copy-id"));
        });
    }

    #[test]
    fn default_options_key_off() {
        assert!(!RemoteOptions::default().install_ssh_key);
    }

    #[test]
    fn remote_probe_arch_ok() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("aarch64\n"));
        let arch = remote_probe_arch(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert_eq!(arch, BoxArch::Arm64);
    }

    #[test]
    fn remote_setup_run_plan_minimal_skip_piper() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("pipeline ok\n"));
        let log = remote_setup_run(
            &runner,
            RemoteOptions {
                host: "box".into(),
                bin_dir: Some(stubs.path().to_path_buf()),
                ..RemoteOptions::default()
            },
            false,
            false,
            true,
            false,
        )
        .unwrap()
        .log;
        assert!(log.contains("pipeline ok") || log.contains("remote command"));
        let cli_ssh = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _, _, _)| p == "ssh")
            .unwrap()
            .1
            .join(" ");
        assert!(!cli_ssh.contains("--apply"));
        assert!(cli_ssh.contains("--skip-piper"));
        assert!(cli_ssh.contains("--minimal"));
    }

    #[test]
    fn remote_run_merges_stderr_and_empty_inherit_log() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            CommandOutput {
                status: 0,
                stdout: "out\n".into(),
                stderr: "warn\n".into(),
            },
        );
        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: false,
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("out"));
        assert!(log.contains("warn"));

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner2);
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("scp", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        let log2 = remote_run_cli(
            &runner2,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: false,
            },
        )
        .unwrap()
        .log;
        assert!(log2.contains("remote command finished"));
    }

    #[test]
    fn remote_install_payload_scp_and_custom_prefix() {
        let stubs = bin_dir_with_stubs();
        let bins = LocalBins {
            dir: stubs.path().to_path_buf(),
            cli: stubs.path().join("horto-os-ui"),
            tui: stubs.path().join("horto-os-ui-tui"),
            status_api: stubs.path().join("horto-os-ui-status-api"),
            mcp: stubs.path().join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        // mkdir staging
        runner.push("ssh", ScriptedRunner::ok(""));
        // prefer_rsync false
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        // scp_files mkdir + 4 scp
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // one enable (token+units+bins[+restart]) + cat drop + rm drop
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            ScriptedRunner::ok("HORTO_API_TOKEN=deadbeefcafebabe\n"),
        );
        runner.push("ssh", ScriptedRunner::ok(""));

        let token = remote_install_payload(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                install_dir: "/opt/horto/bin".into(),
                ..RemoteOptions::default()
            },
            &bins,
        )
        .unwrap();
        assert_eq!(token.as_deref(), Some("deadbeefcafebabe"));
        let ssh_cmds: Vec<String> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _, _)| p == "ssh")
            .map(|(_, args, _, _)| args.join(" "))
            .collect();
        assert!(ssh_cmds
            .iter()
            .any(|c| c.contains("horto-os-ui-status-api.service")
                && c.contains("horto-os-ui-mcp.service")
                && c.contains("/opt/horto/bin")
                && c.contains("restart horto-os-ui-mcp.service")));
        assert!(ssh_cmds.len() >= 5);
    }

    #[test]
    fn remote_run_with_payload_install() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("done\n"));
        // payload
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // enable (token+units+bins) + cat drop + rm
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("aabbccddeeff0011\n"));
        runner.push("ssh", ScriptedRunner::ok(""));

        let outcome = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["setup".into(), "run".into(), "--full".into()],
                use_sudo: true,
                install_payload_on_success: true,
                offer_reboot_on_success: false,
                capture_output: false,
            },
        )
        .unwrap();
        assert!(outcome.log.contains("done"));
        assert_eq!(outcome.api_token.as_deref(), Some("aabbccddeeff0011"));
    }

    #[test]
    fn parse_api_token_drop_accepts_prefix_and_bare_hex() {
        assert_eq!(
            parse_api_token_drop("HORTO_API_TOKEN=abc123\n").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parse_api_token_drop("  deadbeef  \n").as_deref(),
            Some("deadbeef")
        );
        assert!(parse_api_token_drop("").is_none());
        assert!(parse_api_token_drop("not-hex!").is_none());
    }

    #[test]
    fn finish_save_api_token_writes_under_xdg_config() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        assert!(!finish_save_api_token("aabb", "n").unwrap());
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        assert!(finish_save_api_token("aabbccdd", "y").unwrap());
        let path = tmp.path().join("horto-os-ui").join("api_token");
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.trim(), "aabbccdd");
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn write_api_token_file_uses_home_when_xdg_empty() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("XDG_CONFIG_HOME", "   ");
        std::env::set_var("HOME", tmp.path());
        let path = write_api_token_file("11223344").unwrap();
        assert_eq!(
            path,
            tmp.path()
                .join(".config")
                .join("horto-os-ui")
                .join("api_token")
        );
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "11223344");
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn write_api_token_file_uses_home_when_xdg_unset() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let prev_home = std::env::var_os("HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", tmp.path());
        let path = write_api_token_file("55667788").unwrap();
        assert!(path.ends_with("horto-os-ui/api_token"));
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "55667788");
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn offer_save_api_token_non_tty_skips_write() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        assert!(!offer_save_api_token_with("99aabbcc", false, None).unwrap());
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        // Public wrapper still exercises is_terminal() + dispatch.
        let _ = offer_save_api_token("99aabbcc");
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn offer_save_api_token_tty_canned_yes_and_no() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        assert!(!offer_save_api_token_with("aa11", true, Some("n")).unwrap());
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        assert!(offer_save_api_token_with("bb22cc33", true, Some("yes")).unwrap());
        let body = fs::read_to_string(tmp.path().join("horto-os-ui").join("api_token")).unwrap();
        assert_eq!(body.trim(), "bb22cc33");
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn parse_api_token_drop_rejects_empty_hex_value() {
        assert!(parse_api_token_drop("HORTO_API_TOKEN=\n").is_none());
        assert!(parse_api_token_drop("HORTO_API_TOKEN= \n").is_none());
        assert_eq!(
            parse_api_token_drop("\n\nHORTO_API_TOKEN=abcdef\n").as_deref(),
            Some("abcdef")
        );
    }

    #[test]
    fn write_api_token_file_errors_when_config_parent_blocked() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("blocked");
        fs::write(&blocker, b"not-a-directory").unwrap();
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &blocker);
        let err = write_api_token_file("dead").unwrap_err();
        assert!(err.to_string().contains("create"));
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn remote_install_payload_default_prefix_none_token() {
        let stubs = bin_dir_with_stubs();
        let bins = LocalBins {
            dir: stubs.path().to_path_buf(),
            cli: stubs.path().join("horto-os-ui"),
            tui: stubs.path().join("horto-os-ui-tui"),
            status_api: stubs.path().join("horto-os-ui-status-api"),
            mcp: stubs.path().join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // enable (token+units+bins) + empty drop + rm (default /usr/local/bin: no restart)
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("\n"));
        runner.push("ssh", ScriptedRunner::ok(""));

        let token = remote_install_payload(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            &bins,
        )
        .unwrap();
        assert!(token.is_none());
    }

    #[test]
    fn remote_setup_run_full_apply_captures_token() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("setup ok\n"));
        // payload
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            ScriptedRunner::ok("HORTO_API_TOKEN=ffeeddccbbaa9988\n"),
        );
        runner.push("ssh", ScriptedRunner::ok(""));

        let outcome = remote_setup_run(
            &runner,
            RemoteOptions {
                host: "box".into(),
                bin_dir: Some(stubs.path().to_path_buf()),
                ..RemoteOptions::default()
            },
            true,
            true,
            false,
            true,
        )
        .unwrap();
        assert!(outcome.log.contains("setup ok"));
        assert_eq!(outcome.api_token.as_deref(), Some("ffeeddccbbaa9988"));
        let cli_ssh = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|(p, args, _, _)| p == "ssh" && args.iter().any(|a| a.contains("--full")))
            .map(|(_, args, _, _)| args.join(" "))
            .unwrap_or_default();
        assert!(cli_ssh.contains("--full"));
        assert!(cli_ssh.contains("--apply"));
    }

    #[test]
    fn remote_run_outcome_default_is_empty() {
        let o = RemoteRunOutcome::default();
        assert!(o.log.is_empty());
        assert!(o.api_token.is_none());
    }

    #[test]
    fn wants_reboot_now_parses_answers() {
        assert!(wants_reboot_now("y"));
        assert!(wants_reboot_now("YES"));
        assert!(!wants_reboot_now(""));
        assert!(!wants_reboot_now("n"));
        assert!(!wants_reboot_now("maybe"));
    }

    fn test_session() -> SshSession {
        session_from(&RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        })
        .unwrap()
    }

    #[test]
    fn finish_reboot_no_skips_ssh() {
        let runner = ScriptedRunner::default();
        finish_remote_reboot(&runner, &test_session(), "n", None).unwrap();
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn finish_reboot_yes_runs_sudo_reboot() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        finish_remote_reboot(&runner, &test_session(), "yes", None).unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.iter().any(|a| a.contains("sudo reboot")));
    }

    #[test]
    fn finish_reboot_yes_treats_ssh_drop_as_ok() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection closed"));
        finish_remote_reboot(&runner, &test_session(), "y", None).unwrap();
    }

    #[test]
    fn finish_reboot_with_password_feeds_sudo_dash_s() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        finish_remote_reboot(&runner, &test_session(), "y", Some("pw")).unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.iter().any(|a| a.contains("sudo -S reboot")));
        assert!(!calls[0].1.iter().any(|a| *a == "-tt"));
    }

    #[test]
    fn finish_reboot_with_password_propagates_sudo_failure() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(1, "Sorry, try again."));
        let err = finish_remote_reboot(&runner, &test_session(), "y", Some("bad")).unwrap_err();
        assert!(err.to_string().contains("Sorry") || err.to_string().contains("exit 1"));
    }

    #[test]
    fn finish_reboot_with_password_treats_ssh_drop_as_ok() {
        for stderr in [
            "Connection closed by remote host",
            "Connection reset by peer",
            "Broken pipe",
            "ssh: exit 255",
        ] {
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::fail(255, stderr));
            finish_remote_reboot(&runner, &test_session(), "y", Some("pw")).unwrap();
        }
    }

    #[test]
    fn remote_reboot_with_sudo_password_wrapper() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        remote_reboot_with_sudo_password(&runner, &opts, "secret").unwrap();
        let calls = runner.calls.lock().unwrap();
        assert!(calls[0].1.iter().any(|a| a.contains("sudo -S reboot")));
    }

    #[test]
    fn remote_ensure_ssh_key_and_reboot_wrappers() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let runner = ScriptedRunner::default();
            // Probe: already authorized → skip ssh-copy-id.
            runner.push("ssh", ScriptedRunner::ok(""));
            let opts = RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            };
            remote_ensure_ssh_key(&runner, &opts).unwrap();

            let runner2 = ScriptedRunner::default();
            runner2.push("ssh", ScriptedRunner::ok(""));
            remote_reboot(&runner2, &opts).unwrap();
        });
    }

    #[test]
    fn remote_run_offers_reboot_on_success_non_tty() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("done\n"));
        remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: true,
                capture_output: false,
            },
        )
        .unwrap();
    }
}
