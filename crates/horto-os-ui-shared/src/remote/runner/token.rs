//! Local status-api bearer token save path and prompts.

use super::reboot::wants_reboot_now;
use crate::error::Result;
use std::path::PathBuf;

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

pub fn api_token_config_path() -> PathBuf {
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
pub fn offer_save_api_token_with(
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
