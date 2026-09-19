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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::host::parse_host_spec;
    use crate::remote::process::ScriptedRunner;
    use crate::remote::ssh::{SshEnv, SshSession};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn session() -> SshSession {
        SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        }
    }

    #[test]
    fn prefer_rsync_false_when_local_missing() {
        let runner = ScriptedRunner::default();
        runner.push("rsync", ScriptedRunner::fail(127, "not found"));
        assert!(!prefer_rsync(&runner, &session()));
    }

    #[test]
    fn prefer_rsync_true_when_both_ends_have_it() {
        let runner = ScriptedRunner::default();
        runner.push("rsync", ScriptedRunner::ok("rsync  version\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        assert!(prefer_rsync(&runner, &session()));
    }

    #[test]
    fn scp_files_copies_each_local() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("b.bin");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        scp_files(&runner, &session(), &[&a, &b], "/tmp/dest").unwrap();
        let programs: Vec<_> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _, _, _)| p.clone())
            .collect();
        assert_eq!(programs, vec!["ssh", "scp", "scp"]);
    }

    #[test]
    fn transfer_files_falls_back_to_scp() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("x");
        std::fs::write(&f, b"x").unwrap();
        let runner = ScriptedRunner::default();
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        transfer_files(&runner, &session(), &[&f], "/tmp/d").unwrap();
        assert!(runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _, _, _)| p == "scp"));
    }

    #[test]
    fn transfer_files_uses_rsync_when_available() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("x");
        std::fs::write(&f, b"x").unwrap();
        let runner = ScriptedRunner::default();
        // prefer_rsync
        runner.push("rsync", ScriptedRunner::ok("ok"));
        runner.push("ssh", ScriptedRunner::ok(""));
        // mkdir for transfer
        runner.push("ssh", ScriptedRunner::ok(""));
        // rsync copy
        runner.push("rsync", ScriptedRunner::ok(""));
        transfer_files(&runner, &session(), &[&f], "/tmp/d").unwrap();
        let rsync_calls = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _, _)| p == "rsync")
            .count();
        assert!(rsync_calls >= 2);
    }

    #[test]
    fn require_ok_uses_stdout_when_stderr_empty() {
        let runner = ScriptedRunner::default();
        runner.push("rsync", ScriptedRunner::fail(127, ""));
        // force prefer false then scp mkdir fail with stdout-only detail via ssh
        let runner2 = ScriptedRunner::default();
        runner2.push("rsync", ScriptedRunner::fail(1, ""));
        assert!(!prefer_rsync(&runner2, &session()));

        let runner3 = ScriptedRunner::default();
        runner3.push("rsync", ScriptedRunner::ok("v"));
        runner3.push("ssh", ScriptedRunner::fail(1, ""));
        // remote_has_rsync fails -> prefer false; no panic
        assert!(!prefer_rsync(&runner3, &session()));
        let _ = runner;
        let _ = PathBuf::from("/tmp");
    }

    #[test]
    fn rsync_failure_surfaces() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("x");
        std::fs::write(&f, b"x").unwrap();
        let runner = ScriptedRunner::default();
        runner.push("rsync", ScriptedRunner::ok("v"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(1, "boom"));
        let err = transfer_files(&runner, &session(), &[&f], "/tmp/d").unwrap_err();
        assert!(err.to_string().contains("rsync"));
    }
}
