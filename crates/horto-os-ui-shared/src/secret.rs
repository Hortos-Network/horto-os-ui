//! Secret handling helpers (sudo passwords): wipe memory, never log.

/// Overwrite a secret [`String`]'s bytes then clear it.
///
/// Call after the last use of a password buffer (stdin feed, IPC copy).
pub fn wipe_secret(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
    drop(bytes);
}

/// Replace every occurrence of `secret` in `text` (when secret is non-empty).
///
/// Skips secrets shorter than 2 chars to avoid nuking common letters.
#[must_use]
pub fn redact_secret(text: &str, secret: &str) -> String {
    if secret.len() < 2 {
        return text.to_owned();
    }
    text.replace(secret, "********")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipe_secret_empties() {
        let mut s = String::from("hunter2");
        wipe_secret(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn redact_replaces_and_skips_short() {
        assert_eq!(redact_secret("pw=sekret ok", "sekret"), "pw=******** ok");
        assert_eq!(redact_secret("ab x", "a"), "ab x");
    }
}
