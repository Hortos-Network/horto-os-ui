//! Map box `uname -m` to GitHub Release target triples.

use crate::error::{HortoError, Result};

/// Box architecture labels used for Release assets and local cache paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxArch {
    /// `x86_64` / amd64.
    Amd64,
    /// `aarch64` / arm64.
    Arm64,
}

impl BoxArch {
    /// Rust target triple for Release tar.gz assets.
    #[must_use]
    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::Amd64 => "x86_64-unknown-linux-gnu",
            Self::Arm64 => "aarch64-unknown-linux-gnu",
        }
    }

    /// Short arch label used in cache directories.
    #[must_use]
    pub const fn cache_label(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

/// Parse `uname -m` output into a supported box architecture.
///
/// # Errors
///
/// Returns [`crate::HortoError::Message`] when the machine string is empty or unsupported.
pub fn box_arch_from_uname(uname_m: &str) -> Result<BoxArch> {
    let trimmed = uname_m.trim();
    match trimmed {
        "x86_64" | "amd64" => Ok(BoxArch::Amd64),
        "aarch64" | "arm64" => Ok(BoxArch::Arm64),
        "" => Err(HortoError::msg("empty uname -m from remote host")),
        other => Err(HortoError::msg(format!(
            "unsupported box architecture: {other} (need x86_64 or aarch64)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_unames() {
        assert_eq!(box_arch_from_uname("x86_64\n").unwrap(), BoxArch::Amd64);
        assert_eq!(box_arch_from_uname("amd64").unwrap(), BoxArch::Amd64);
        assert_eq!(box_arch_from_uname("aarch64").unwrap(), BoxArch::Arm64);
        assert_eq!(box_arch_from_uname("arm64").unwrap(), BoxArch::Arm64);
    }

    #[test]
    fn rejects_unknown() {
        assert!(box_arch_from_uname("riscv64").is_err());
        assert!(box_arch_from_uname("").is_err());
    }

    #[test]
    fn triples_match_release_assets() {
        assert_eq!(BoxArch::Amd64.target_triple(), "x86_64-unknown-linux-gnu");
        assert_eq!(BoxArch::Arm64.target_triple(), "aarch64-unknown-linux-gnu");
        assert_eq!(BoxArch::Amd64.cache_label(), "amd64");
        assert_eq!(BoxArch::Arm64.cache_label(), "arm64");
    }
}
