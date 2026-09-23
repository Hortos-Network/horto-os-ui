//! Remote box reboot helpers.

use super::super::process::{ProcessRunner, StdioMode};
use super::super::ssh::SshSession;
use super::options::{remote_progress, session_from, RemoteOptions};
use crate::error::Result;

/// Issue `sudo reboot` on the box (CLI: password on the terminal via Inherit).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails before reboot starts.
pub fn remote_reboot(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Result<()> {
    let session = session_from(opts)?;
    finish_remote_reboot(runner, &session, "y", None)
}

/// Issue `sudo reboot` using a sudo password already collected by the UI.
///
/// Feeds `sudo -S` over captured SSH. Used by the TUI (no cooked TTY).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails before reboot starts.
pub fn remote_reboot_with_sudo_password(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    sudo_password: &str,
) -> Result<()> {
    let session = session_from(opts)?;
    finish_remote_reboot(runner, &session, "y", Some(sudo_password))
}

/// Whether a reboot prompt answer means reboot now.
#[must_use]
pub fn wants_reboot_now(raw: &str) -> bool {
    matches!(raw.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Prompt on a local TTY, then `sudo reboot` on the box. Non-TTY prints a reminder.
pub fn offer_remote_reboot(runner: &dyn ProcessRunner, session: &SshSession) -> Result<()> {
    use std::io::{self, IsTerminal, Write};

    if !io::stdin().is_terminal() {
        eprintln!("Reboot recommended for hostname/network changes. On the box: sudo reboot");
        return Ok(());
    }
    eprint!("Reboot the box now to apply hostname/network changes? [y/N]: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| crate::error::HortoError::msg(format!("read reboot prompt: {e}")))?;
    finish_remote_reboot(runner, session, &line, None)
}

pub fn finish_remote_reboot(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    answer: &str,
    sudo_password: Option<&str>,
) -> Result<()> {
    if !wants_reboot_now(answer) {
        eprintln!("Skipping reboot. Reboot the box later when convenient.");
        return Ok(());
    }
    remote_progress(
        &session.host.raw,
        "reboot box (SSH + sudo; may ask password)",
    );
    eprintln!("Rebooting...");
    let used_stdin = sudo_password.is_some();
    let result = sudo_password.map_or_else(
        || session.exec(runner, "sudo reboot", StdioMode::Inherit),
        |pass| {
            let mut feed = String::with_capacity(pass.len() + 1);
            feed.push_str(pass);
            feed.push('\n');
            let out = session.exec_stdin_reboot(runner, "sudo -S reboot", feed.as_bytes());
            feed.clear();
            out
        },
    );
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            if used_stdin && !ssh_drop_after_reboot(&e) {
                return Err(e);
            }
            // Host drop mid-session is expected once reboot starts.
            eprintln!("reboot issued (SSH session closed is expected): {e}");
            Ok(())
        }
    }
}

pub fn ssh_drop_after_reboot(err: &crate::error::HortoError) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("connection closed")
        || s.contains("connection reset")
        || s.contains("broken pipe")
        || s.contains("exit 255")
}
