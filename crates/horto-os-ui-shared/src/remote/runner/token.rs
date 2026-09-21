//! Local status-api bearer token save path and prompts.

use super::reboot::wants_reboot_now;
use crate::error::{HortoError, Result};
use std::path::PathBuf;

/// Minimum tip bearer length. Box install writes 64 hex chars; shorter leftovers
/// (including unit-test junk) must never be treated as a real Status API token.
pub const MIN_API_TOKEN_HEX_LEN: usize = 32;

/// Return trimmed hex when it is usable as a tip Status API bearer.
///
/// Accepts `HORTO_API_TOKEN=<hex>` or bare hex. Rejects empty, non-hex, and
/// anything shorter than [`MIN_API_TOKEN_HEX_LEN`].
#[must_use]
pub fn usable_api_token_hex(raw: &str) -> Option<&str> {
    let hex = raw.strip_prefix("HORTO_API_TOKEN=").unwrap_or(raw).trim();
    if hex.len() < MIN_API_TOKEN_HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        None
    } else {
        Some(hex)
    }
}

/// Parse drop-file contents: `HORTO_API_TOKEN=<hex>` or bare hex.
#[must_use]
pub fn parse_api_token_drop(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    usable_api_token_hex(line).map(str::to_owned)
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
/// Rejects short / non-hex input **before** creating or truncating the tip file,
/// so a bad caller cannot wipe a real PC token with junk.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the token is unusable, or the directory /
/// file cannot be written.
pub fn write_api_token_file(token: &str) -> Result<PathBuf> {
    use std::fs;
    use std::io::Write;

    let hex = usable_api_token_hex(token).ok_or_else(|| {
        HortoError::msg(format!(
            "status-api bearer must be at least {MIN_API_TOKEN_HEX_LEN} hex characters"
        ))
    })?;

    let path = api_token_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| HortoError::msg(format!("create {}: {e}", parent.display())))?;
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
                .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
            writeln!(f, "{hex}")
                .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&path, format!("{hex}\n"))
                .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
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
            .map_err(|e| HortoError::msg(format!("read token save prompt: {e}")))?;
        line
    };
    finish_save_api_token(token, &line)
}
