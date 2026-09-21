//! Local Release cache and `--bin-dir` resolution for box binaries.

use super::arch::BoxArch;
use super::process::{CommandOutput, ProcessRunner, StdioMode};
use crate::error::{HortoError, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Binaries that must be present for remote / embedded install to proceed.
pub const REQUIRED_BOX_BIN_NAMES: [&str; 3] =
    ["horto-os-ui", "horto-os-ui-tui", "horto-os-ui-status-api"];

/// Filenames expected in a full box Release tar (MCP may be absent).
pub const BOX_BIN_NAMES: [&str; 4] = [
    "horto-os-ui",
    "horto-os-ui-tui",
    "horto-os-ui-status-api",
    "horto-os-ui-mcp",
];

/// Paths to box binaries after cache extract or `--bin-dir`.
#[derive(Debug, Clone)]
pub struct LocalBins {
    /// Directory that contains the binaries (source or install layout).
    pub dir: PathBuf,
    /// CLI apply agent.
    pub cli: PathBuf,
    /// TUI binary.
    pub tui: PathBuf,
    /// Status API binary.
    pub status_api: PathBuf,
    /// MCP server binary when present in the Release / bin dir.
    pub mcp: Option<PathBuf>,
}

fn warn_mcp_binary_missing(dir: &Path) {
    let msg = format!(
        "horto-os-ui-mcp not found under {} — continuing without MCP on the box",
        dir.display()
    );
    tracing::warn!("{msg}");
    eprintln!("[horto remote] warning: {msg}");
}

/// Release tarball file name for a version + target triple.
#[must_use]
pub fn asset_name(version: &str, arch: BoxArch) -> String {
    format!("horto-os-ui-{version}-{}.tar.gz", arch.target_triple())
}

/// Default GitHub Release tag for a Cargo version (`0.1.0` → `v0.1.0`).
#[must_use]
pub fn default_release_tag(version: &str) -> String {
    format!("v{version}")
}

/// GitHub Release download URL for a box tar.gz asset.
///
/// `release_tag` is the GitHub tag (`v0.1.0` or tip Pre-release `dev-preview`).
/// The asset **filename** still embeds `version` (Cargo workspace version).
#[must_use]
pub fn release_download_url(repo: &str, release_tag: &str, version: &str, arch: BoxArch) -> String {
    let name = asset_name(version, arch);
    format!("https://github.com/{repo}/releases/download/{release_tag}/{name}")
}

/// Whether a Release tag is treated as immutable for the local remote-bins cache.
///
/// Stable tags look like `v0.1.0` (leading `v` + digit). Tip tags such as
/// `dev-preview` are overwritten in place with the same asset filenames, so a
/// warm cache would keep stale tip binaries forever.
#[must_use]
pub fn release_tag_is_immutable(release_tag: &str) -> bool {
    let mut chars = release_tag.chars();
    matches!(chars.next(), Some('v')) && matches!(chars.next(), Some(c) if c.is_ascii_digit())
}

/// Default XDG cache root: `$XDG_CACHE_HOME/horto-os-ui/remote-bins` or `~/.cache/...`.
#[must_use]
pub fn default_cache_root() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("horto-os-ui").join("remote-bins")
}

/// Cache directory for one release tag + version + arch.
#[must_use]
pub fn cache_bin_dir(
    cache_root: &Path,
    release_tag: &str,
    version: &str,
    arch: BoxArch,
) -> PathBuf {
    cache_root
        .join(release_tag)
        .join(version)
        .join(arch.cache_label())
}

fn bins_from_dir(dir: &Path) -> Result<LocalBins> {
    let cli = dir.join("horto-os-ui");
    let tui = dir.join("horto-os-ui-tui");
    let status_api = dir.join("horto-os-ui-status-api");
    let mcp_path = dir.join("horto-os-ui-mcp");
    for p in [&cli, &tui, &status_api] {
        if !p.is_file() {
            return Err(HortoError::msg(format!(
                "missing binary {} (required: {})",
                p.display(),
                REQUIRED_BOX_BIN_NAMES.join(", ")
            )));
        }
    }
    let mcp = if mcp_path.is_file() {
        Some(mcp_path)
    } else {
        warn_mcp_binary_missing(dir);
        None
    };
    Ok(LocalBins {
        dir: dir.to_path_buf(),
        cli,
        tui,
        status_api,
        mcp,
    })
}

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

fn which_bin(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

/// Resolve ecosystem binaries for embedded install.
///
/// Prefers a complete sibling directory next to the running executable, then a
/// complete `install_dir`, then a merge of sibling / `install_dir` / `PATH`.
/// MCP is optional: missing `horto-os-ui-mcp` warns and continues.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when CLI, TUI, or status-api cannot be found.
pub fn resolve_local_ecosystem_bins(install_dir: &Path) -> Result<LocalBins> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Ok(bins) = bins_from_dir(dir) {
                return Ok(bins);
            }
        }
    }
    if let Ok(bins) = bins_from_dir(install_dir) {
        return Ok(bins);
    }

    let sibling = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    let dir = sibling.clone().unwrap_or_else(|| install_dir.to_path_buf());

    let pick = |name: &str| -> Result<PathBuf> {
        let mut cands = Vec::new();
        if let Some(ref sib) = sibling {
            cands.push(sib.join(name));
        }
        cands.push(install_dir.join(name));
        if let Some(p) = which_bin(name) {
            cands.push(p);
        }
        first_existing(&cands).ok_or_else(|| {
            HortoError::msg(format!(
                "missing binary {name} (looked next to exe, in {}, and on PATH)",
                install_dir.display()
            ))
        })
    };

    let mcp = {
        let mut cands = Vec::new();
        if let Some(ref sib) = sibling {
            cands.push(sib.join("horto-os-ui-mcp"));
        }
        cands.push(install_dir.join("horto-os-ui-mcp"));
        if let Some(p) = which_bin("horto-os-ui-mcp") {
            cands.push(p);
        }
        first_existing(&cands).or_else(|| {
            warn_mcp_binary_missing(&dir);
            None
        })
    };

    Ok(LocalBins {
        dir,
        cli: pick("horto-os-ui")?,
        tui: pick("horto-os-ui-tui")?,
        status_api: pick("horto-os-ui-status-api")?,
        mcp,
    })
}

fn require_ok(program: &str, out: &CommandOutput) -> Result<()> {
    if out.success() {
        return Ok(());
    }
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_owned()
    } else {
        out.stderr.trim().to_owned()
    };
    Err(HortoError::command(
        program,
        format!("exit {}: {detail}", out.status),
    ))
}

/// Resolve box binaries from `--bin-dir` or download+extract a Release tar.gz.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when paths are missing, download fails, or extract fails.
pub fn ensure_local_bins(
    runner: &dyn ProcessRunner,
    release_tag: &str,
    version: &str,
    repo: &str,
    arch: BoxArch,
    bin_dir: Option<&Path>,
    cache_root: &Path,
) -> Result<LocalBins> {
    if let Some(dir) = bin_dir {
        return bins_from_dir(dir);
    }

    let dest = cache_bin_dir(cache_root, release_tag, version, arch);
    let marker = dest.join("horto-os-ui");
    if marker.is_file() && release_tag_is_immutable(release_tag) {
        return bins_from_dir(&dest);
    }

    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    let url = release_download_url(repo, release_tag, version, arch);
    let tarball = dest.join(asset_name(version, arch));
    let curl_out = runner.run(
        "curl",
        &[
            "-fsSL",
            "-o",
            tarball
                .to_str()
                .ok_or_else(|| HortoError::msg("non-utf8 path"))?,
            &url,
        ],
        &[],
        StdioMode::Capture,
    )?;
    require_ok("curl", &curl_out)?;

    let dest_str = dest
        .to_str()
        .ok_or_else(|| HortoError::msg("non-utf8 cache path"))?;
    let tar_out = runner.run(
        "tar",
        &[
            "-xzf",
            tarball
                .to_str()
                .ok_or_else(|| HortoError::msg("non-utf8 tarball path"))?,
            "-C",
            dest_str,
            "--strip-components=1",
        ],
        &[],
        StdioMode::Capture,
    )?;
    require_ok("tar", &tar_out)?;
    bins_from_dir(&dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::process::{CommandOutput, ProcessRunner, ScriptedRunner, StdioMode};
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Scripted runner that materializes box binaries when `tar` is invoked.
    struct ExtractRunner {
        inner: ScriptedRunner,
        dest: PathBuf,
        payload: &'static [u8],
    }

    impl ProcessRunner for ExtractRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            env: &[(&str, &str)],
            stdio: StdioMode,
        ) -> crate::error::Result<CommandOutput> {
            let out = self.inner.run(program, args, env, stdio)?;
            if program == "tar" {
                fs::create_dir_all(&self.dest).unwrap();
                for name in [
                    "horto-os-ui",
                    "horto-os-ui-tui",
                    "horto-os-ui-status-api",
                    "horto-os-ui-mcp",
                ] {
                    fs::write(self.dest.join(name), self.payload).unwrap();
                }
            }
            Ok(out)
        }
    }

    #[test]
    fn asset_and_url_shape() {
        let name = asset_name("0.1.0", BoxArch::Amd64);
        assert_eq!(name, "horto-os-ui-0.1.0-x86_64-unknown-linux-gnu.tar.gz");
        assert_eq!(default_release_tag("0.1.0"), "v0.1.0");
        let url = release_download_url(
            "Hortos-Network/horto-os-ui",
            "v0.1.0",
            "0.1.0",
            BoxArch::Arm64,
        );
        assert!(url.contains("/download/v0.1.0/"));
        assert!(url.ends_with("aarch64-unknown-linux-gnu.tar.gz"));
        let tip = release_download_url(
            "Hortos-Network/horto-os-ui",
            "dev-preview",
            "0.1.0",
            BoxArch::Amd64,
        );
        assert!(tip.contains("/download/dev-preview/"));
        assert!(tip.contains("horto-os-ui-0.1.0-x86_64-unknown-linux-gnu.tar.gz"));
        assert!(release_tag_is_immutable("v0.1.0"));
        assert!(release_tag_is_immutable("v1.2.3-rc.1"));
        assert!(!release_tag_is_immutable("dev-preview"));
        assert!(!release_tag_is_immutable("latest"));
        assert!(!release_tag_is_immutable("0.1.0"));
    }

    #[test]
    fn bin_dir_requires_core_three_mcp_optional() {
        let tmp = TempDir::new().unwrap();
        let err = bins_from_dir(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("missing binary"));

        for name in REQUIRED_BOX_BIN_NAMES {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let bins = bins_from_dir(tmp.path()).unwrap();
        assert!(bins.cli.ends_with("horto-os-ui"));
        assert!(bins.mcp.is_none());

        fs::write(tmp.path().join("horto-os-ui-mcp"), b"x").unwrap();
        let bins = bins_from_dir(tmp.path()).unwrap();
        assert!(bins
            .mcp
            .as_ref()
            .is_some_and(|p| p.ends_with("horto-os-ui-mcp")));
    }

    #[test]
    fn ensure_uses_bin_dir_without_curl() {
        let tmp = TempDir::new().unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let runner = ScriptedRunner::default();
        let bins = ensure_local_bins(
            &runner,
            "v0.1.0",
            "0.1.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Amd64,
            Some(tmp.path()),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(bins.dir, tmp.path());
        assert_eq!(runner.calls.lock().unwrap().as_slice(), &[]);
    }

    #[test]
    fn ensure_downloads_when_cache_cold() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache");
        let runner = ScriptedRunner::default();
        runner.push("curl", ScriptedRunner::ok(""));
        runner.push("tar", ScriptedRunner::ok(""));

        // Pre-create binaries as if tar extracted them (tar is scripted as success).
        let dest = cache_bin_dir(&cache, "v0.1.0", "0.1.0", BoxArch::Amd64);
        fs::create_dir_all(&dest).unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(dest.join(name), b"x").unwrap();
        }

        // Cold cache: delete marker so download path runs, but recreate after curl/tar scripts.
        // Simpler: run ensure with empty dest and have tar script also write files via side effect
        // before bins_from_dir - we already wrote files; ensure checks marker first.
        // Wipe and re-run download path:
        let _ = fs::remove_dir_all(&dest);
        fs::create_dir_all(&dest).unwrap();
        // After curl+tar, ensure calls bins_from_dir - write files now simulating tar.
        // Race: ensure creates dest, curls, tars, then bins_from_dir. We need files after tar.
        // Scripted tar just returns ok - so write files in a custom way:
        // Call ensure when cache already has files after we push scripts that "extract".
        // Re-write approach: push curl/tar, and between them we can't inject. So write files
        // before ensure, but then marker exists and skip download.
        // Instead test download failure and success of curl args only via a unit that
        // doesn't need extract:

        let runner2 = ScriptedRunner::default();
        runner2.push("curl", ScriptedRunner::fail(22, "404"));
        let err = ensure_local_bins(
            &runner2,
            "v0.1.0",
            "0.1.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Amd64,
            None,
            &cache,
        )
        .unwrap_err();
        assert!(err.to_string().contains("curl"));
        let calls = runner2.calls.lock().unwrap();
        assert_eq!(calls[0].0, "curl");
        assert!(calls[0].1.iter().any(|a| a.contains("github.com")));
        drop(calls);
    }

    #[test]
    fn ensure_uses_warm_cache_without_download() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache");
        let dest = cache_bin_dir(&cache, "v0.1.0", "0.1.0", BoxArch::Amd64);
        fs::create_dir_all(&dest).unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(dest.join(name), b"x").unwrap();
        }
        let runner = ScriptedRunner::default();
        let bins = ensure_local_bins(
            &runner,
            "v0.1.0",
            "0.1.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Amd64,
            None,
            &cache,
        )
        .unwrap();
        assert_eq!(bins.dir, dest);
        assert_eq!(runner.calls.lock().unwrap().as_slice(), &[]);
    }

    #[test]
    fn ensure_tip_tag_redownloads_over_warm_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache");
        let dest = cache_bin_dir(&cache, "dev-preview", "0.1.0", BoxArch::Arm64);
        fs::create_dir_all(&dest).unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(dest.join(name), b"stale").unwrap();
        }

        let inner = ScriptedRunner::default();
        inner.push("curl", ScriptedRunner::ok(""));
        inner.push("tar", ScriptedRunner::ok(""));
        let runner = ExtractRunner {
            inner,
            dest: dest.clone(),
            payload: b"fresh",
        };
        let bins = ensure_local_bins(
            &runner,
            "dev-preview",
            "0.1.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Arm64,
            None,
            &cache,
        )
        .unwrap();
        assert_eq!(bins.dir, dest);
        assert_eq!(fs::read(dest.join("horto-os-ui")).unwrap(), b"fresh");
        let calls = runner.inner.calls.lock().unwrap();
        assert_eq!(calls[0].0, "curl");
        assert!(calls[0]
            .1
            .iter()
            .any(|a| a.contains("/download/dev-preview/")));
        assert_eq!(calls[1].0, "tar");
        drop(calls);
    }

    #[test]
    fn ensure_download_and_extract_success() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache");
        let dest = cache_bin_dir(&cache, "dev-preview", "0.2.0", BoxArch::Arm64);
        let inner = ScriptedRunner::default();
        inner.push("curl", ScriptedRunner::ok(""));
        inner.push("tar", ScriptedRunner::ok(""));
        let extract = ExtractRunner {
            inner,
            dest,
            payload: b"x",
        };
        let bins = ensure_local_bins(
            &extract,
            "dev-preview",
            "0.2.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Arm64,
            None,
            &cache,
        )
        .unwrap();
        assert!(bins.cli.is_file());
    }

    #[test]
    fn tar_failure_uses_stderr_detail() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache2");
        let runner = ScriptedRunner::default();
        runner.push("curl", ScriptedRunner::ok(""));
        runner.push("tar", ScriptedRunner::fail(2, "bad archive"));
        let err = ensure_local_bins(
            &runner,
            "v0.3.0",
            "0.3.0",
            "Hortos-Network/horto-os-ui",
            BoxArch::Amd64,
            None,
            &cache,
        )
        .unwrap_err();
        assert!(err.to_string().contains("tar"));
        assert!(err.to_string().contains("bad archive"));
    }

    #[test]
    fn default_cache_root_non_empty() {
        assert_ne!(default_cache_root().as_os_str(), "");
    }

    #[test]
    fn resolve_local_ecosystem_bins_from_install_dir() {
        let tmp = TempDir::new().unwrap();
        for name in BOX_BIN_NAMES {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let bins = resolve_local_ecosystem_bins(tmp.path()).unwrap();
        assert_eq!(bins.dir, tmp.path());
        assert!(bins
            .mcp
            .as_ref()
            .is_some_and(|p| p.ends_with("horto-os-ui-mcp")));
    }

    #[test]
    fn resolve_local_ecosystem_bins_ok_without_mcp() {
        let tmp = TempDir::new().unwrap();
        for name in REQUIRED_BOX_BIN_NAMES {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let bins = resolve_local_ecosystem_bins(tmp.path()).unwrap();
        assert!(bins.mcp.is_none());
        assert!(bins.cli.is_file());
    }

    #[test]
    fn resolve_local_ecosystem_bins_errors_when_incomplete() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("horto-os-ui"), b"x").unwrap();
        let err = resolve_local_ecosystem_bins(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn cache_bin_dir_layout() {
        let root = PathBuf::from("/tmp/cache");
        let p = cache_bin_dir(&root, "v0.1.0", "0.1.0", BoxArch::Amd64);
        assert!(p.to_string_lossy().contains("v0.1.0"));
        assert!(p.to_string_lossy().contains("0.1.0"));
    }
}
