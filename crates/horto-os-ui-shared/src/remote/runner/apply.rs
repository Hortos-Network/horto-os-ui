//! Prepare / upload remote CLI, run commands, and install payload.

use super::super::arch::box_arch_from_uname;
use super::super::bins::{ensure_local_bins, LocalBins};
use super::super::ecosystem::{
    remote_enable_ecosystem_script, EcosystemInstallChoice, API_TOKEN_DROP_BASENAME,
};
use super::super::process::{ProcessRunner, StdioMode};
use super::super::ssh::SshSession;
use super::super::transfer::transfer_files;
use super::options::{
    build_remote_command_at, merge_command_log, remote_agent_bin, remote_progress,
    remote_run_banner_detail, session_from, shell_quote, RemoteOptions, RemoteRunFlags,
    RemoteRunRequest,
};
use super::probe::{probe_remote_cli, RemoteCliProbe};
use super::reboot::offer_remote_reboot;
use super::token::parse_api_token_drop;
use crate::error::{HortoError, Result};
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
    let remote_cmd = build_remote_command_at(&remote_bin, &cli_args, req.use_sudo);
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

    let install_payload = req.install_payload_on_success && req.ecosystem.any();
    let api_token = if install_payload {
        // Always push tip bins from the PC after apply. Do not gate on probe.current:
        // Desktop tip SHA often differs from published Release bins; skipping left the
        // box on stale /usr/local/bin while the agent already skipped ecosystem install.
        remote_install_payload(runner, opts, &bins, req.ecosystem)?
    } else {
        None
    };
    if req.offer_reboot_on_success {
        offer_remote_reboot(runner, &session)?;
    }
    Ok(RemoteRunOutcome { log, api_token })
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
                offer_reboot_on_success: apply,
                capture_output,
                allow_stale_cli,
            },
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
        "install box payload (SCP/SSH; may ask password)",
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
        "enable selected ecosystem services on box (SSH + sudo; may ask password)",
    );
    let enable = remote_enable_ecosystem_script(&staging, install, choice);
    session.exec(runner, &enable, StdioMode::Inherit)?;

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
    }

    Ok(api_token)
}
