//! System askpass (`SSH_ASKPASS`) for secrets when there is no cooked TTY.

use crate::error::{HortoError, Result};
use std::process::Stdio;

/// Run `$SSH_ASKPASS` with `prompt` and return the secret (trimmed, not logged).
///
/// # Errors
///
/// Returns [`HortoError`] when `SSH_ASKPASS` is unset/empty, the program fails,
/// or the secret is empty (cancelled).
pub fn prompt_secret(prompt: &str) -> Result<String> {
    let ask = std::env::var("SSH_ASKPASS").map_err(|_| {
        HortoError::msg("SSH_ASKPASS is not set (required for TUI/Desktop secrets)")
    })?;
    if ask.trim().is_empty() {
        return Err(HortoError::msg("SSH_ASKPASS is empty"));
    }
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let out = std::process::Command::new(&ask)
        .arg(prompt)
        .env("DISPLAY", display)
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| HortoError::command(&ask, e.to_string()))?;
    if !out.status.success() {
        return Err(HortoError::msg("askpass cancelled or failed"));
    }
    let mut secret = String::from_utf8_lossy(&out.stdout).into_owned();
    while secret.ends_with(['\n', '\r']) {
        secret.pop();
    }
    if secret.is_empty() {
        return Err(HortoError::msg("askpass returned an empty secret"));
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
    fn prompt_secret_errors_when_unset() {
        let _g = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("SSH_ASKPASS");
        let err = prompt_secret("x").unwrap_err().to_string();
        assert!(err.contains("SSH_ASKPASS"));
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
