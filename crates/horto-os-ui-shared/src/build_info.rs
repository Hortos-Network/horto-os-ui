//! Package version and git commit baked at compile time.

/// Workspace package version (`Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git SHA from build (`unknown` if unavailable); may end with `-dirty`.
pub const GIT_COMMIT: &str = env!("GIT_COMMIT_HASH");

/// Static label for clap `long_version` and UI footers: `0.1.0 (abc1234)`.
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_COMMIT_HASH"),
    ")"
);

/// Human footer line: `v0.1.0 · abc1234`.
#[must_use]
pub fn footer_line() -> String {
    format!("v{VERSION} · {GIT_COMMIT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_nonempty() {
        assert!(!VERSION.is_empty());
        assert!(!GIT_COMMIT.is_empty());
        assert!(LONG_VERSION.contains(VERSION));
        assert!(footer_line().starts_with('v'));
    }
}
