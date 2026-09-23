//! Remote session options, banners, and shared command helpers.

use super::super::ecosystem::{EcosystemInstallChoice, DEFAULT_INSTALL_DIR as ECO_INSTALL_DIR};
use super::super::host::parse_host_spec;
use super::super::process::{ProcessRunner, StdioMode};
use super::super::ssh::SshSession;
use crate::error::Result;
use crate::VERSION;
use std::path::PathBuf;

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
        let release_tag = super::super::bins::default_release_tag(&version);
        Self {
            host: String::new(),
            install_ssh_key: false,
            bin_dir: None,
            version,
            release_tag,
            github_repo: DEFAULT_GITHUB_REPO.to_owned(),
            cache_root: super::super::bins::default_cache_root(),
            remote_agent_dir: DEFAULT_REMOTE_AGENT_DIR.to_owned(),
            install_dir: DEFAULT_INSTALL_DIR.to_owned(),
            ssh_config_file: None,
        }
    }
}

/// Common inputs shared by CLI, TUI, desktop, and MCP when building [`RemoteOptions`].
#[derive(Debug, Clone, Default)]
pub struct RemoteOptionsInput {
    /// OpenSSH Host alias or `user@host`.
    pub host: String,
    /// When true, run `ssh-copy-id` once before apply.
    pub install_ssh_key: bool,
    /// Local directory with the four box binaries (skips GitHub download).
    pub bin_dir: Option<PathBuf>,
    /// Override Release tag when non-empty after trim; otherwise keep default.
    pub release_tag: Option<String>,
}

impl RemoteOptions {
    /// Build session options from shared surface inputs (host / key / bin / tag).
    #[must_use]
    pub fn from_input(input: RemoteOptionsInput) -> Self {
        let mut opts = Self {
            host: input.host,
            install_ssh_key: input.install_ssh_key,
            bin_dir: input.bin_dir,
            ..Self::default()
        };
        if let Some(tag) = input
            .release_tag
            .as_ref()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
        {
            tag.clone_into(&mut opts.release_tag);
        }
        opts
    }
}

/// Boolean switches for a remote CLI run.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteRunFlags {
    /// Prefix with `sudo -n` / `sudo` when true (apply mode).
    pub use_sudo: bool,
    /// After a successful command, install selected ecosystem services on the box.
    pub install_payload_on_success: bool,
    /// After success (and payload install), offer an interactive box reboot (TTY only).
    pub offer_reboot_on_success: bool,
    /// Capture remote stdout/stderr instead of inheriting the local TTY.
    ///
    /// Use for read-only status/doctor so callers can parse JSON. Keep false for
    /// apply paths that need interactive sudo / password prompts.
    pub capture_output: bool,
    /// When true, run even if the box CLI long-version does not match tip
    /// [`crate::LONG_VERSION`]. Default false (same gate as TUI s0).
    pub allow_stale_cli: bool,
}

/// CLI arguments to run on the box via the uploaded agent (without the binary name).
#[derive(Debug, Clone)]
pub struct RemoteRunRequest {
    /// Session and binary options.
    pub options: RemoteOptions,
    /// Arguments after the remote `horto-os-ui` binary (e.g. `["setup", "run", "--full"]` or with `--apply`).
    pub cli_args: Vec<String>,
    /// Sudo / payload / reboot / capture switches.
    pub flags: RemoteRunFlags,
    /// Which services to install when [`RemoteRunFlags::install_payload_on_success`] is set.
    pub ecosystem: EcosystemInstallChoice,
}

impl std::ops::Deref for RemoteRunRequest {
    type Target = RemoteRunFlags;

    fn deref(&self) -> &Self::Target {
        &self.flags
    }
}

pub fn session_from(opts: &RemoteOptions) -> Result<SshSession> {
    Ok(SshSession {
        host: parse_host_spec(&opts.host)?,
        config_file: opts.ssh_config_file.clone(),
    })
}

pub fn shell_quote(arg: &str) -> String {
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

pub fn remote_progress(host: &str, detail: &str) {
    tracing::info!("{}", remote_progress_message(host, detail));
}

/// Log the remote doctor JSON header (stderr via tracing).
pub fn remote_doctor_report_banner() {
    tracing::info!("[horto remote] doctor report from box (JSON):");
}

pub fn remote_agent_bin(opts: &RemoteOptions) -> String {
    format!(
        "{}/horto-os-ui",
        opts.remote_agent_dir.trim_end_matches('/')
    )
}

pub fn remote_install_bin(opts: &RemoteOptions) -> String {
    format!("{}/horto-os-ui", opts.install_dir.trim_end_matches('/'))
}

pub fn remote_cli_candidates(opts: &RemoteOptions) -> [String; 2] {
    [remote_install_bin(opts), remote_agent_bin(opts)]
}

pub fn build_remote_command_at(bin: &str, cli_args: &[String], use_sudo: bool) -> String {
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

pub fn merge_command_log(out: &super::super::process::CommandOutput, remote_cmd: &str) -> String {
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
pub fn exec_remote_cli_captured(
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
