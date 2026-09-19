//! OpenSSH session helpers (`ssh`, `scp`, `ssh-copy-id`).

use super::host::HostSpec;
use super::process::{CommandOutput, ProcessRunner, StdioMode};
use crate::error::{HortoError, Result};
use std::path::{Path, PathBuf};

/// Environment overrides for Desktop askpass (and related OpenSSH knobs).
#[derive(Debug, Clone, Default)]
pub struct SshEnv {
    /// When true, set `SSH_ASKPASS_REQUIRE=force` and pass through `SSH_ASKPASS` if set.
    pub force_askpass: bool,
}

impl SshEnv {
    /// Environment key/value pairs for OpenSSH child processes.
    #[must_use]
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if self.force_askpass {
            out.push(("SSH_ASKPASS_REQUIRE".into(), "force".into()));
            if let Ok(ask) = std::env::var("SSH_ASKPASS") {
                if !ask.is_empty() {
                    out.push(("SSH_ASKPASS".into(), ask));
                    out.push((
                        "DISPLAY".into(),
                        std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
                    ));
                }
            }
        }
        out
    }

    fn as_refs(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
        pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

/// SSH destination plus askpass policy.
#[derive(Debug, Clone)]
pub struct SshSession {
    /// OpenSSH Host or `user@host`.
    pub host: HostSpec,
    /// Askpass / env policy.
    pub env: SshEnv,
    /// Optional `ssh -F` / `scp -F` config file (tests / non-default layouts).
    pub config_file: Option<PathBuf>,
}

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

/// First existing default OpenSSH public key under `$HOME/.ssh`.
fn default_identity_pubkey() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| HortoError::msg("HOME unset; cannot find default SSH identity"))?;
    let ssh_dir = home.join(".ssh");
    for name in ["id_ed25519.pub", "id_ecdsa.pub", "id_rsa.pub"] {
        let path = ssh_dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(HortoError::msg(format!(
        "no default SSH pubkey in {} (tried id_ed25519.pub, id_ecdsa.pub, id_rsa.pub)",
        ssh_dir.display()
    )))
}

/// Private key path next to a `.pub` file (`id_ed25519.pub` → `id_ed25519`).
fn identity_private_key(pub_path: &Path) -> Option<PathBuf> {
    let stem = pub_path.file_stem()?;
    let priv_path = pub_path.parent()?.join(stem);
    priv_path.is_file().then_some(priv_path)
}

impl SshSession {
    fn with_config_prefix(&self, rest: &[&str]) -> Vec<String> {
        let mut owned = Vec::new();
        if let Some(cfg) = &self.config_file {
            owned.push("-F".into());
            owned.push(cfg.display().to_string());
        }
        for a in rest {
            owned.push((*a).to_owned());
        }
        owned
    }

    /// True when BatchMode SSH with this identity already succeeds (key is on the box).
    fn pubkey_already_authorized(
        &self,
        runner: &dyn ProcessRunner,
        pub_path: &Path,
    ) -> Result<bool> {
        let Some(priv_path) = identity_private_key(pub_path) else {
            return Ok(false);
        };
        let priv_s = priv_path.display().to_string();
        let mut pairs = self.env.as_pairs();
        // Do not offer other agent keys during the probe.
        pairs.push(("SSH_AUTH_SOCK".into(), String::new()));
        let env = SshEnv::as_refs(&pairs);
        let owned = self.with_config_prefix(&[
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ConnectTimeout=10",
            "-i",
            &priv_s,
            &self.host.raw,
            "true",
        ]);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let out = runner.run("ssh", &refs, &env, StdioMode::Capture)?;
        Ok(out.success())
    }

    /// Run a remote shell command via `ssh`.
    ///
    /// Probes use [`StdioMode::Capture`]. Interactive apply uses inherit so
    /// OpenSSH / sudo can prompt on a TTY.
    ///
    /// # Errors
    ///
    /// Returns [`crate::HortoError::CommandFailed`] when ssh exits non-zero.
    pub fn exec(
        &self,
        runner: &dyn ProcessRunner,
        remote_cmd: &str,
        stdio: StdioMode,
    ) -> Result<CommandOutput> {
        let pairs = self.env.as_pairs();
        let env = SshEnv::as_refs(&pairs);
        let owned = self.with_config_prefix(&[
            "-o",
            "BatchMode=no",
            "-o",
            "StrictHostKeyChecking=accept-new",
            &self.host.raw,
            remote_cmd,
        ]);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let out = runner.run("ssh", &refs, &env, stdio)?;
        require_ok("ssh", &out)?;
        Ok(out)
    }

    /// Copy a local file to a remote path with `scp`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::HortoError`] when paths are invalid or scp fails.
    pub fn scp_to(
        &self,
        runner: &dyn ProcessRunner,
        local: &Path,
        remote_path: &str,
    ) -> Result<()> {
        let local_s = local
            .to_str()
            .ok_or_else(|| HortoError::msg("non-utf8 local path for scp"))?;
        let dest = format!("{}:{remote_path}", self.host.raw);
        let pairs = self.env.as_pairs();
        let env = SshEnv::as_refs(&pairs);
        let owned =
            self.with_config_prefix(&["-o", "StrictHostKeyChecking=accept-new", local_s, &dest]);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let out = runner.run("scp", &refs, &env, StdioMode::Inherit)?;
        require_ok("scp", &out)
    }

    /// Opt-in: install the default identity pubkey on the box via `ssh-copy-id -i`.
    ///
    /// Skips `ssh-copy-id` when BatchMode SSH with that identity already works
    /// (key already authorized). Without `-i`, `ssh-copy-id` would install every
    /// agent key; we always pass a single `.pub`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::HortoError`] when no default pubkey exists or `ssh-copy-id` fails.
    pub fn install_ssh_key(&self, runner: &dyn ProcessRunner) -> Result<()> {
        let pub_path = default_identity_pubkey()?;
        if self.pubkey_already_authorized(runner, &pub_path)? {
            eprintln!(
                "[horto remote] pubkey {} already authorized on {}; skip ssh-copy-id",
                pub_path.display(),
                self.host.raw
            );
            return Ok(());
        }
        let pub_s = pub_path.display().to_string();
        let pairs = self.env.as_pairs();
        let env = SshEnv::as_refs(&pairs);
        let owned = self.with_config_prefix(&[
            "-i",
            &pub_s,
            "-o",
            "StrictHostKeyChecking=accept-new",
            &self.host.raw,
        ]);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let out = runner.run("ssh-copy-id", &refs, &env, StdioMode::Inherit)?;
        require_ok("ssh-copy-id", &out)
    }

    /// Probe whether `rsync` exists on the remote host.
    pub fn remote_has_rsync(&self, runner: &dyn ProcessRunner) -> bool {
        self.exec(
            runner,
            "command -v rsync >/dev/null 2>&1",
            StdioMode::Capture,
        )
        .is_ok()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::remote::host::parse_host_spec;
    use crate::remote::process::ScriptedRunner;

    #[test]
    fn exec_builds_ssh_args() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        };
        let out = session
            .exec(&runner, "uname -m", StdioMode::Capture)
            .unwrap();
        assert_eq!(out.stdout.trim(), "x86_64");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0, "ssh");
        assert!(calls[0].1.iter().any(|a| a == "box"));
        assert!(calls[0].1.iter().any(|a| a == "uname -m"));
    }

    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn home_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    /// Serialize HOME mutation for SSH identity discovery tests.
    fn with_temp_home<R>(setup: impl FnOnce(&Path), f: impl FnOnce() -> R) -> R {
        let _guard = home_lock();

        let dir = tempfile::TempDir::new().unwrap();
        setup(dir.path());

        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        let out = f();
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    /// HOME + `.ssh/id_ed25519` (+ `.pub`) for install_ssh_key tests.
    pub(crate) fn with_fake_default_pubkey<R>(f: impl FnOnce(PathBuf) -> R) -> R {
        let _guard = home_lock();
        let dir = tempfile::TempDir::new().unwrap();
        let ssh = dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let pub_path = ssh.join("id_ed25519.pub");
        std::fs::write(&pub_path, "ssh-ed25519 AAAATEST test@ci\n").unwrap();
        std::fs::write(ssh.join("id_ed25519"), b"PRIVATE\n").unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        let out = f(pub_path);
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    #[test]
    fn default_identity_errors_when_home_unset() {
        let _guard = home_lock();
        let prev = std::env::var_os("HOME");
        std::env::remove_var("HOME");
        let err = default_identity_pubkey().unwrap_err();
        if let Some(h) = prev {
            std::env::set_var("HOME", h);
        }
        assert!(err.to_string().contains("HOME unset"));
    }

    #[test]
    fn default_identity_errors_when_no_pubkey() {
        with_temp_home(
            |home| {
                std::fs::create_dir_all(home.join(".ssh")).unwrap();
            },
            || {
                let err = default_identity_pubkey().unwrap_err();
                assert!(err.to_string().contains("no default SSH pubkey"));
            },
        );
    }

    #[test]
    fn default_identity_falls_back_to_ecdsa_then_rsa() {
        with_temp_home(
            |home| {
                let ssh = home.join(".ssh");
                std::fs::create_dir_all(&ssh).unwrap();
                std::fs::write(ssh.join("id_ecdsa.pub"), "ecdsa-sha2-nistp256 AAAA ecdsa\n")
                    .unwrap();
            },
            || {
                let path = default_identity_pubkey().unwrap();
                assert!(path.ends_with("id_ecdsa.pub"));
            },
        );
        with_temp_home(
            |home| {
                let ssh = home.join(".ssh");
                std::fs::create_dir_all(&ssh).unwrap();
                std::fs::write(ssh.join("id_rsa.pub"), "ssh-rsa AAAA rsa\n").unwrap();
            },
            || {
                let path = default_identity_pubkey().unwrap();
                assert!(path.ends_with("id_rsa.pub"));
            },
        );
    }

    #[test]
    fn install_key_passes_i_pubkey() {
        with_fake_default_pubkey(|pub_path| {
            let runner = ScriptedRunner::default();
            // Probe: key not yet authorized.
            runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
            runner.push("ssh-copy-id", ScriptedRunner::ok(""));
            let session = SshSession {
                host: parse_host_spec("box").unwrap(),
                env: SshEnv::default(),
                config_file: None,
            };
            session.install_ssh_key(&runner).unwrap();
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls[0].0, "ssh");
            assert!(calls[0].1.iter().any(|a| a == "BatchMode=yes"));
            assert_eq!(calls[1].0, "ssh-copy-id");
            let args = &calls[1].1;
            let i = args.iter().position(|a| a == "-i").expect("-i missing");
            assert_eq!(Path::new(&args[i + 1]), pub_path.as_path());
            assert!(args.iter().any(|a| a == "box"));
        });
    }

    #[test]
    fn install_key_skips_when_already_authorized() {
        with_fake_default_pubkey(|_| {
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::ok(""));
            let session = SshSession {
                host: parse_host_spec("box").unwrap(),
                env: SshEnv::default(),
                config_file: None,
            };
            session.install_ssh_key(&runner).unwrap();
            let programs: Vec<_> = runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(p, _, _, _)| p.clone())
                .collect();
            assert_eq!(programs, vec!["ssh"]);
            assert!(!programs.iter().any(|p| p == "ssh-copy-id"));
        });
    }

    #[test]
    fn install_key_errors_when_no_default_pubkey() {
        with_temp_home(
            |home| {
                std::fs::create_dir_all(home.join(".ssh")).unwrap();
            },
            || {
                let runner = ScriptedRunner::default();
                let session = SshSession {
                    host: parse_host_spec("box").unwrap(),
                    env: SshEnv::default(),
                    config_file: None,
                };
                let err = session.install_ssh_key(&runner).unwrap_err();
                assert!(err.to_string().contains("no default SSH pubkey"));
            },
        );
    }

    #[test]
    fn scp_failure_surfaces() {
        let runner = ScriptedRunner::default();
        runner.push("scp", ScriptedRunner::fail(1, "Permission denied"));
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        };
        let err = session
            .scp_to(&runner, Path::new("/tmp/x"), "/tmp/x")
            .unwrap_err();
        assert!(err.to_string().contains("scp"));
    }

    #[test]
    fn config_file_adds_f_flag() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("ok\n"));
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: Some(PathBuf::from("/tmp/ssh_config")),
        };
        session.exec(&runner, "true", StdioMode::Capture).unwrap();
        let args = &runner.calls.lock().unwrap()[0].1;
        assert_eq!(args[0], "-F");
        assert_eq!(args[1], "/tmp/ssh_config");
    }

    #[test]
    fn force_askpass_sets_env() {
        std::env::set_var("SSH_ASKPASS", "/usr/bin/ssh-askpass");
        let env = SshEnv {
            force_askpass: true,
        };
        let pairs = env.as_pairs();
        assert!(pairs.iter().any(|(k, _)| k == "SSH_ASKPASS_REQUIRE"));
        assert!(pairs.iter().any(|(k, _)| k == "SSH_ASKPASS"));
        std::env::remove_var("SSH_ASKPASS");
    }

    #[test]
    fn require_ok_prefers_stdout_when_stderr_blank() {
        let runner = ScriptedRunner::default();
        runner.push(
            "ssh",
            CommandOutput {
                status: 1,
                stdout: "denied-stdout".into(),
                stderr: String::new(),
            },
        );
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        };
        let err = session
            .exec(&runner, "false", StdioMode::Capture)
            .unwrap_err();
        assert!(err.to_string().contains("denied-stdout"));
    }

    #[test]
    fn remote_has_rsync_true_and_false() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        };
        assert!(session.remote_has_rsync(&runner));
        runner.push("ssh", ScriptedRunner::fail(1, "no"));
        assert!(!session.remote_has_rsync(&runner));
    }

    #[test]
    fn scp_ok_path() {
        let runner = ScriptedRunner::default();
        runner.push("scp", ScriptedRunner::ok(""));
        let session = SshSession {
            host: parse_host_spec("box").unwrap(),
            env: SshEnv::default(),
            config_file: None,
        };
        session
            .scp_to(&runner, Path::new("/tmp/x"), "/tmp/x")
            .unwrap();
    }
}
