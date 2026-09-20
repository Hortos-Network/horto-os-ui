//! System askpass (`SSH_ASKPASS`) for secrets when there is no cooked TTY.

use crate::error::{HortoError, Result};
use std::path::PathBuf;
use std::process::Stdio;

/// Resolve the askpass program: `$SSH_ASKPASS`, else a common name on `PATH`.
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
        "No askpass program found. Set SSH_ASKPASS to an askpass binary on PATH.",
    ))
}

/// Run the system askpass with `prompt` and return the secret (trimmed, not logged).
///
/// # Errors
///
/// Returns [`HortoError`] when no askpass is available, the program fails, or the
/// secret is empty (cancelled).
pub fn prompt_secret(prompt: &str) -> Result<String> {
    let ask = resolve_askpass()?;
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let out = std::process::Command::new(&ask)
        .arg(prompt)
        .env("DISPLAY", display)
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| HortoError::command(ask.display().to_string(), e.to_string()))?;
    if !out.status.success() {
        return Err(HortoError::msg("Askpass cancelled or failed"));
    }
    let mut secret = String::from_utf8_lossy(&out.stdout).into_owned();
    while secret.ends_with(['\n', '\r']) {
        secret.pop();
    }
    if secret.is_empty() {
        return Err(HortoError::msg("Askpass returned an empty secret"));
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::ENV_LOCK;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn prompt_secret_errors_when_unset_and_no_path_askpass() {
        let _g = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("SSH_ASKPASS");
        // Prepend an empty PATH so which() cannot find a host askpass.
        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let err = prompt_secret("x").unwrap_err().to_string();
        match old_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
        assert!(
            err.contains("No askpass program found"),
            "unexpected err: {err}"
        );
        assert!(
            !err.contains("TUI/Desktop secrets"),
            "junk phrase must stay gone: {err}"
        );
        assert!(
            !err.contains("eisonot"),
            "must be readable English with spaces: {err}"
        );
    }

    #[test]
    fn prompt_secret_reads_askpass_stdout() {
        let _g = ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tmpdir");
        let script = dir.path().join("askpass.sh");
        fs::write(&script, "#!/bin/sh\necho -n 'test-secret'\n").expect("write");
        let mut perms = fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");
        std::env::set_var("SSH_ASKPASS", &script);
        let got = prompt_secret("Sudo?").expect("askpass");
        std::env::remove_var("SSH_ASKPASS");
        assert_eq!(got, "test-secret");
    }
}
