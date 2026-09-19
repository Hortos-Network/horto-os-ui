//! File transfer: `scp` first; prefer `rsync -e ssh` when both ends have it.

use super::process::{CommandOutput, ProcessRunner, StdioMode};
use super::ssh::SshSession;
use crate::error::{HortoError, Result};
use std::path::Path;

fn require_ok(program: &str, out: &CommandOutput) -> Result<()> {
    if out.success() {
        return Ok(());
    }
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_owned()
    } else {
        out.stderr.trim().to_owned()
    };
    Err(HortoError::command(
        program,
        format!("exit {}: {detail}", out.status),
    ))
}

/// True when local `rsync` exists and the remote host has `rsync` on PATH.
pub fn prefer_rsync(runner: &dyn ProcessRunner, session: &SshSession) -> bool {
    let local = runner
        .run("rsync", &["--version"], &[], StdioMode::Capture)
        .map(|o| o.success())
        .unwrap_or(false);
    local && session.remote_has_rsync(runner)
}

/// Copy one or more local files to a remote directory using `scp`.
///
/// # Errors
///
/// Returns [`HortoError`] when scp fails.
pub fn scp_files(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    locals: &[&Path],
    remote_dir: &str,
) -> Result<()> {
    session.exec(
        runner,
        &format!("mkdir -p {remote_dir}"),
        StdioMode::Capture,
    )?;
    for local in locals {
        let name = local
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| HortoError::msg("bad local file name for scp"))?;
        let remote_path = format!("{remote_dir}/{name}");
        session.scp_to(runner, local, &remote_path)?;
    }
    Ok(())
}

/// Copy files with rsync when available, otherwise scp.
///
/// # Errors
///
/// Returns [`HortoError`] when transfer fails.
pub fn transfer_files(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    locals: &[&Path],
    remote_dir: &str,
) -> Result<()> {
    if prefer_rsync(runner, session) {
        session.exec(
            runner,
            &format!("mkdir -p {remote_dir}"),
            StdioMode::Capture,
        )?;
        let dest = format!("{}:{remote_dir}/", session.host.raw);
        let mut args: Vec<&str> = vec!["-e", "ssh", "-a"];
        let path_strs: Vec<String> = locals
            .iter()
            .map(|p| {
                p.to_str()
                    .ok_or_else(|| HortoError::msg("non-utf8 path for rsync"))
                    .map(str::to_owned)
            })
            .collect::<Result<Vec<_>>>()?;
        for s in &path_strs {
            args.push(s.as_str());
        }
        args.push(&dest);
        let env_owned = session.env.as_pairs();
        let env_refs: Vec<(&str, &str)> = env_owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let out = runner.run("rsync", &args, &env_refs, StdioMode::Inherit)?;
        require_ok("rsync", &out)?;
        return Ok(());
    }
    scp_files(runner, session, locals, remote_dir)
}
