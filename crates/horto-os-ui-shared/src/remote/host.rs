//! OpenSSH host target: config Host alias or `user@host`.

use crate::error::{HortoError, Result};

/// Destination passed to `ssh` / `scp` as the remote operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    /// Raw value suitable for `ssh <spec>` (Host alias or `user@host`).
    pub raw: String,
}

/// Parse a non-empty Host alias or `user@host` string.
///
/// # Errors
///
/// Returns [`crate::HortoError::Message`] when the string is empty or only whitespace.
pub fn parse_host_spec(input: &str) -> Result<HostSpec> {
    let raw = input.trim().to_owned();
    if raw.is_empty() {
        return Err(HortoError::msg(
            "remote host is empty; pass an OpenSSH Host alias or user@host",
        ));
    }
    if raw.contains(char::is_whitespace) {
        return Err(HortoError::msg(format!(
            "remote host must not contain whitespace: {raw:?}"
        )));
    }
    Ok(HostSpec { raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_alias_and_user_host() {
        assert_eq!(parse_host_spec(" horto-box ").unwrap().raw, "horto-box");
        assert_eq!(
            parse_host_spec("greg@192.168.1.10").unwrap().raw,
            "greg@192.168.1.10"
        );
    }

    #[test]
    fn rejects_empty_or_spaces() {
        assert!(parse_host_spec("").is_err());
        assert!(parse_host_spec("  ").is_err());
        assert!(parse_host_spec("bad host").is_err());
    }
}
