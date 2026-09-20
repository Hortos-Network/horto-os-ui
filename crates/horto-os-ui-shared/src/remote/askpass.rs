//! Resolve an OpenSSH askpass binary for Desktop / non-TTY SSH login.

use crate::error::{HortoError, Result};
use std::path::PathBuf;

/// Resolve `$SSH_ASKPASS`, else a common askpass name on `PATH`.
///
/// Used only to populate OpenSSH's environment for SSH *login* when
/// [`super::ssh::SshEnv::force_askpass`] is set. Sudo reboot does not use this.
///
/// # Errors
///
/// Returns [`HortoError`] when neither the env var nor a known binary is found.
pub fn resolve_askpass() -> Result<PathBuf> {
    if let Ok(ask) = std::env::var("SSH_ASKPASS") {
        let ask = ask.trim();
        if !ask.is_empty() {
            return Ok(PathBuf::from(ask));
        }
    }
    for name in [
        "ssh-askpass",
        "x11-ssh-askpass",
        "gnome-ssh-askpass",
        "ksshaskpass",
        "lxqt-openssh-askpass",
    ] {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }
    Err(HortoError::msg(
        "No system password helper found for SSH login",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::ENV_LOCK;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn resolve_askpass_errors_when_unset_and_no_path() {
        let _g = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("SSH_ASKPASS");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let err = resolve_askpass().unwrap_err().to_string();
        match old_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
        assert!(err.contains("No system password helper found"));
    }

    #[test]
    fn resolve_askpass_prefers_env() {
        let _g = ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tmpdir");
        let script = dir.path().join("askpass.sh");
        fs::write(&script, "#!/bin/sh\n").expect("write");
        let mut perms = fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");
        std::env::set_var("SSH_ASKPASS", &script);
        let got = resolve_askpass().expect("resolve");
        std::env::remove_var("SSH_ASKPASS");
        assert_eq!(got, script);
    }
}
