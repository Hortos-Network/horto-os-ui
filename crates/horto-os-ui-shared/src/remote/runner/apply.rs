//! Prepare / upload remote CLI, run commands, and install payload.

use super::super::arch::box_arch_from_uname;
use super::super::bins::{ensure_local_bins, LocalBins};
use super::super::ecosystem::{
    remote_enable_ecosystem_script, EcosystemInstallChoice, API_TOKEN_DROP_BASENAME,
};
use super::super::process::{CommandOutput, ProcessRunner, StdioMode};
use super::super::ssh::SshSession;
use super::super::transfer::transfer_files;
use super::options::{
    build_remote_command_sudo, merge_command_log, remote_agent_bin, remote_progress,
    remote_run_banner_detail, remote_sudo_kind, session_from, shell_quote, RemoteOptions,
    RemoteRunFlags, RemoteRunRequest, RemoteSudoKind,
};
use super::probe::{normalize_cli_version, probe_remote_cli, RemoteCliProbe};
use super::reboot::offer_remote_reboot;
use super::token::parse_api_token_drop;
use crate::error::{HortoError, Result};
use crate::secret::{redact_secret, wipe_secret};
use crate::LONG_VERSION;
use std::path::Path;

pub fn upload_remote_cli(
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
    remote_progress(
        &opts.host,
        "upload horto CLI to the box (SCP; may ask password)",
    );
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
pub fn ensure_remote_cli(
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

pub fn maybe_install_key(
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
pub fn prepare_remote_agent(
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

/// Result of a remote CLI run (log text plus optional captured status-api bearer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRunOutcome {
    /// Merged remote stdout/stderr (or a short finished marker when Inherit was empty).
    pub log: String,
    /// Hex bearer from payload install, when captured from the box drop file.
    pub api_token: Option<String>,
}

/// Drop tip-only globals so an older box agent can still parse argv.
#[must_use]
pub fn strip_tip_only_cli_flags(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| {
            let s = a.as_str();
            !(s == "--install-status-api"
                || s == "--no-install-status-api"
                || s == "--install-mcp"
                || s == "--no-install-mcp"
                || s == "--stacks"
                || s.starts_with("--stacks="))
        })
        .cloned()
        .collect()
}

fn stale_cli_error(box_label: &str) -> HortoError {
    HortoError::msg(format!(
        "Box CLI is out of date (box={box_label}, tip={LONG_VERSION}). Sync CLI first, refresh tip release or set HORTO_BIN_DIR to matching bins, or allow an old CLI."
    ))
}

/// Upload the CLI agent (if needed), optionally install a public key, run a remote CLI command.
///
/// Refuses when the box CLI long-version does not match tip [`LONG_VERSION`], unless
/// [`RemoteRunFlags::allow_stale_cli`] is set. With allow + stale, tip-only argv flags
/// (`--install-*` / `--stacks`) are stripped so older agents can still parse.
///
/// # Errors
///
/// Returns [`crate::HortoError`] on SSH/SCP/agent failures or a stale CLI without allow.
pub fn remote_run_cli(
    runner: &dyn ProcessRunner,
    req: &RemoteRunRequest,
) -> Result<RemoteRunOutcome> {
    let opts = &req.options;
    let (session, bins, remote_bin) = prepare_remote_agent(runner, opts)?;
    let probe = probe_remote_cli(runner, &session, opts)?;
    if !probe.current && !req.allow_stale_cli {
        let label = probe
            .version
            .as_deref()
            .unwrap_or_else(|| probe.status.as_label());
        return Err(stale_cli_error(label));
    }

    let cli_args = if probe.current {
        req.cli_args.clone()
    } else {
        strip_tip_only_cli_flags(&req.cli_args)
    };
    let sudo_kind = remote_sudo_kind(req.use_sudo, req.sudo_password.as_deref());
    let remote_cmd = build_remote_command_sudo(&remote_bin, &cli_args, sudo_kind);
    remote_progress(
        &opts.host,
        &remote_run_banner_detail(&remote_cmd, req.use_sudo),
    );
    // Desktop (sudo_password Some) never inherits the launch TTY.
    let capture = req.capture_output || req.sudo_password.is_some();
    let out = match sudo_kind {
        RemoteSudoKind::Stdin => {
            let pass = req.sudo_password.as_deref().unwrap_or("");
            let mut feed = String::with_capacity(pass.len() + 1);
            feed.push_str(pass);
            feed.push('\n');
            let result = session.exec_stdin(runner, &remote_cmd, feed.as_bytes());
            wipe_secret(&mut feed);
            result?
        }
        RemoteSudoKind::None | RemoteSudoKind::Prompt | RemoteSudoKind::NonInteractive => {
            let stdio = if capture {
                StdioMode::Capture
            } else {
                // Inherit so SSH/sudo password prompts work on a TTY (CLI/TUI apply).
                StdioMode::Inherit
            };
            session.exec(runner, &remote_cmd, stdio)?
        }
    };
    let secret = req.sudo_password.as_deref().unwrap_or("");
    let log = redact_secret(&merge_command_log(&out, &remote_cmd), secret);
    if capture && !log.trim().is_empty() {
        publish_captured_apply_log(&log);
    }

    let install_payload = req.install_payload_on_success && req.ecosystem.any();
    let api_token = if install_payload {
        // Always push tip bins from the PC after apply. Do not gate on probe.current:
        // Desktop tip SHA often differs from published Release bins; skipping left the
        // box on stale /usr/local/bin while the agent already skipped ecosystem install.
        remote_install_payload(
            runner,
            opts,
            &bins,
            req.ecosystem,
            req.sudo_password.as_deref(),
        )?
    } else {
        None
    };
    if req.offer_reboot_on_success {
        offer_remote_reboot(runner, &session)?;
    }
    remote_progress(&opts.host, "remote setup finished");
    Ok(RemoteRunOutcome { log, api_token })
}

/// Print Capture stdout once and mirror into the Desktop [`crate::log_bus::LogBus`] (when installed).
pub fn publish_captured_apply_log(log: &str) {
    let log = log.trim_end();
    if log.is_empty() {
        return;
    }
    eprintln!("{log}");
    crate::log_bus::LogBus::mirror_capture_to_process_bus(log);
}

/// Inputs for [`remote_setup_run`] (avoids a long bool parameter list).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct RemoteSetupRunArgs {
    /// Session and binary options.
    pub options: RemoteOptions,
    /// Privileged apply on the box.
    pub apply: bool,
    /// Full pipeline when true; minimal when false.
    pub full: bool,
    /// Skip piper model download.
    pub skip_piper: bool,
    /// Ecosystem services to install after successful apply.
    pub ecosystem: EcosystemInstallChoice,
    /// Optional Docker stacks for d3.
    pub stack_opts: crate::stack_opts::StackOpts,
    /// Allow setup when box CLI ≠ tip long-version.
    pub allow_stale_cli: bool,
    /// Pipe remote stdout/stderr back to the caller (Desktop / MCP). Leave false for
    /// CLI/TUI so OpenSSH can prompt on a real TTY.
    pub capture_output: bool,
    /// After apply, prompt to reboot the box (CLI/TUI). Desktop leaves this false;
    /// reboot is a new SSH sudo and would ask for the password again.
    pub offer_reboot: bool,
    /// When `Some`, Desktop-style Capture + `sudo -S` / `sudo -n` (never Inherit).
    ///
    /// `None` keeps CLI/TUI interactive sudo on a TTY.
    pub sudo_password: Option<String>,
}

/// Convenience: remote `setup run` with optional payload install.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the remote setup fails or the box CLI is stale
/// without [`RemoteSetupRunArgs::allow_stale_cli`].
pub fn remote_setup_run(
    runner: &dyn ProcessRunner,
    args: RemoteSetupRunArgs,
) -> Result<RemoteRunOutcome> {
    let RemoteSetupRunArgs {
        options: opts,
        apply,
        full,
        skip_piper,
        ecosystem,
        stack_opts,
        allow_stale_cli,
        capture_output,
        offer_reboot,
        sudo_password,
    } = args;
    let mut cli_args = Vec::new();
    if apply {
        cli_args.push("--apply".into());
    }
    if skip_piper {
        cli_args.push("--skip-piper".into());
    }
    let stacks_csv = stack_opts.to_csv();
    if !stacks_csv.is_empty() {
        cli_args.push(format!("--stacks={stacks_csv}"));
    }
    if ecosystem.status_api {
        cli_args.push("--install-status-api".into());
    } else {
        cli_args.push("--no-install-status-api".into());
    }
    if ecosystem.mcp {
        cli_args.push("--install-mcp".into());
    } else {
        cli_args.push("--no-install-mcp".into());
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
            ecosystem,
            flags: RemoteRunFlags {
                use_sudo: apply,
                install_payload_on_success: apply && ecosystem.any(),
                offer_reboot_on_success: offer_reboot,
                capture_output,
                allow_stale_cli,
            },
            sudo_password,
        },
    )
}

/// Copy CLI + TUI + status-api (+ MCP when present) into `install_dir` and enable units.
///
/// Missing `horto-os-ui-mcp` skips MCP transfer/enable; other units still install.
/// Returns the captured hex bearer when the drop file can be read.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when transfer or remote enable fails.
pub fn remote_install_payload(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    bins: &LocalBins,
    choice: EcosystemInstallChoice,
    sudo_password: Option<&str>,
) -> Result<Option<String>> {
    let choice = EcosystemInstallChoice {
        status_api: choice.status_api,
        mcp: choice.mcp && bins.mcp.is_some(),
    };
    if !choice.any() {
        return Ok(None);
    }
    let session = session_from(opts)?;
    let staging = format!("{}/payload", opts.remote_agent_dir.trim_end_matches('/'));
    remote_progress(
        &opts.host,
        "copy apps onto the box (SCP/SSH; may ask password)",
    );
    session.exec(
        runner,
        &format!("mkdir -p {}", shell_quote(&staging)),
        StdioMode::Capture,
    )?;

    let mut locals: Vec<&Path> = vec![&bins.cli, &bins.tui, &bins.status_api];
    if let Some(ref mcp) = bins.mcp {
        locals.push(mcp.as_path());
    }
    transfer_files(runner, &session, &locals, &staging)?;

    let install = opts.install_dir.trim_end_matches('/');
    remote_progress(
        &opts.host,
        "enable status-api and MCP on the box (SSH + sudo; may ask password)",
    );
    let enable = remote_enable_ecosystem_script(&staging, install, choice);
    exec_payload_enable(runner, &session, &enable, sudo_password)?;

    let mut api_token = None;
    if choice.status_api {
        let drop_path = format!("$HOME/{API_TOKEN_DROP_BASENAME}");
        let cat_out = session.exec(
            runner,
            &format!("cat {drop_path} 2>/dev/null || true"),
            StdioMode::Capture,
        )?;
        api_token = parse_api_token_drop(&cat_out.stdout);
        let _ = session.exec(runner, &format!("rm -f {drop_path}"), StdioMode::Capture);
        verify_remote_status_api(runner, &session, install)?;
    }

    Ok(api_token)
}

/// Run the multi-sudo enable script without a TTY.
///
/// With a password: `sudo -S bash -s` so every nested `sudo` in the script inherits
/// the authenticated session (plain `sudo` after `sudo -S -v` still needs a TTY).
fn exec_payload_enable(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    enable: &str,
    sudo_password: Option<&str>,
) -> Result<()> {
    match sudo_password {
        Some("") => {
            session.exec_stdin(runner, "sudo -n bash -s", enable.as_bytes())?;
        }
        Some(pass) => {
            let mut feed =
                String::with_capacity(pass.len().saturating_add(1).saturating_add(enable.len()));
            feed.push_str(pass);
            feed.push('\n');
            feed.push_str(enable);
            let result = session.exec_stdin(runner, "sudo -S -p '' bash -s", feed.as_bytes());
            wipe_secret(&mut feed);
            result?;
        }
        None => {
            session.exec(runner, enable, StdioMode::Inherit)?;
        }
    }
    Ok(())
}

/// Confirm on-disk status-api `--version` matches the running `/health` `cli_version`.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when `--version` fails, health fails, or the two disagree.
fn verify_remote_status_api(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    install_dir: &str,
) -> Result<()> {
    remote_progress(
        &session.host.raw,
        "verify status-api on box (--version vs /health)",
    );
    let bin = format!("{install_dir}/horto-os-ui-status-api");
    let ver_out = session.exec(
        runner,
        &format!("{} --version", shell_quote(&bin)),
        StdioMode::Capture,
    )?;
    let health_out = session.exec(
        runner,
        "curl -fsS --connect-timeout 3 --max-time 8 http://127.0.0.1:8787/health",
        StdioMode::Capture,
    )?;
    evaluate_status_api_verify(bin.as_str(), &ver_out, &health_out).map_err(|e| {
        tracing::error!("status-api verify failed: {e}");
        e
    })?;
    remote_progress(
        &session.host.raw,
        "status-api verify ok (--version matches /health)",
    );
    Ok(())
}

/// Check captured `--version` and `/health` outputs agree on the installed build.
fn evaluate_status_api_verify(
    bin: &str,
    ver_out: &CommandOutput,
    health_out: &CommandOutput,
) -> Result<()> {
    let on_disk = normalize_cli_version(&ver_out.stdout);
    if !ver_out.success() || on_disk.is_empty() {
        return Err(HortoError::msg(format!(
            "status-api install verify failed: `{bin} --version` empty or non-zero (exit {})",
            ver_out.status
        )));
    }
    if !health_out.success() {
        let detail = if health_out.stderr.trim().is_empty() {
            health_out.stdout.trim()
        } else {
            health_out.stderr.trim()
        };
        return Err(HortoError::msg(format!(
            "status-api install verify failed: GET http://127.0.0.1:8787/health (exit {}): {detail}",
            health_out.status
        )));
    }
    let Some(running) = health_cli_version(&health_out.stdout) else {
        return Err(HortoError::msg(format!(
            "status-api install verify failed: /health missing cli_version (got {})",
            health_out.stdout.trim()
        )));
    };
    if !status_api_versions_agree(&on_disk, &running) {
        return Err(HortoError::msg(format!(
            "status-api install verify failed: on-disk `{on_disk}` != running /health cli_version `{running}`"
        )));
    }
    Ok(())
}

/// True when on-disk `--version` and `/health` `cli_version` name the same build.
///
/// Clap prints `horto-os-ui-status-api 0.1.0 (abc)`; health uses `0.1.0 (abc)`.
fn status_api_versions_agree(on_disk: &str, health_cli: &str) -> bool {
    let disk = normalize_cli_version(on_disk);
    let health = normalize_cli_version(health_cli);
    if disk.is_empty() || health.is_empty() {
        return false;
    }
    disk == health || disk.ends_with(&format!(" {health}")) || disk.ends_with(&health)
}

/// Parse non-empty `cli_version` from a status-api `/health` JSON body.
fn health_cli_version(body: &str) -> Option<String> {
    let start = body.find('{')?;
    let value: serde_json::Value = serde_json::from_str(body[start..].trim()).ok()?;
    value
        .get("cli_version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{evaluate_status_api_verify, health_cli_version, status_api_versions_agree};
    use crate::remote::process::CommandOutput;

    fn out(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn status_api_versions_agree_clap_prefix_and_health() {
        assert!(status_api_versions_agree(
            "horto-os-ui-status-api 0.1.0 (deadbeef)",
            "0.1.0 (deadbeef)"
        ));
        assert!(status_api_versions_agree(
            "0.1.0 (deadbeef)",
            "0.1.0 (deadbeef)"
        ));
        assert!(!status_api_versions_agree(
            "horto-os-ui-status-api 0.1.0 (aaaaaaa)",
            "0.1.0 (bbbbbbb)"
        ));
        assert!(!status_api_versions_agree("", "0.1.0 (deadbeef)"));
    }

    #[test]
    fn health_cli_version_reads_field() {
        assert_eq!(
            health_cli_version(r#"{"ok":true,"cli_version":"0.1.0 (abc)"}"#).as_deref(),
            Some("0.1.0 (abc)")
        );
        assert!(health_cli_version(r#"{"ok":true}"#).is_none());
        assert!(health_cli_version(r#"{"ok":true,"cli_version":""}"#).is_none());
    }

    #[test]
    fn evaluate_status_api_verify_ok_and_failures() {
        let ok_ver = out(0, "horto-os-ui-status-api 0.1.0 (deadbeef)\n", "");
        let ok_health = out(0, r#"{"ok":true,"cli_version":"0.1.0 (deadbeef)"}"#, "");
        assert!(evaluate_status_api_verify(
            "/usr/local/bin/horto-os-ui-status-api",
            &ok_ver,
            &ok_health
        )
        .is_ok());

        let bad_ver = out(1, "", "fail");
        let err = evaluate_status_api_verify("bin", &bad_ver, &ok_health).unwrap_err();
        assert!(err.to_string().contains("--version"));

        let empty_ver = out(0, "\n", "");
        let err = evaluate_status_api_verify("bin", &empty_ver, &ok_health).unwrap_err();
        assert!(err.to_string().contains("--version"));

        let bad_health = out(7, "", "curl: fail");
        let err = evaluate_status_api_verify("bin", &ok_ver, &bad_health).unwrap_err();
        assert!(err.to_string().contains("/health"));

        let no_cli = out(0, r#"{"ok":true}"#, "");
        let err = evaluate_status_api_verify("bin", &ok_ver, &no_cli).unwrap_err();
        assert!(err.to_string().contains("cli_version"));

        let mismatch = out(0, r#"{"ok":true,"cli_version":"0.1.0 (other)"}"#, "");
        let err = evaluate_status_api_verify("bin", &ok_ver, &mismatch).unwrap_err();
        assert!(err.to_string().contains("!="));
    }
}
