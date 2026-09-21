//! Filesystem helpers that respect [`HostContext`] dry-run vs apply.

use crate::context::HostContext;
use crate::error::{HortoError, Result};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Ensure `path` exists as a directory (`mkdir -p`).
///
/// In dry-run mode, records a planned action and returns without creating.
///
/// # Errors
///
/// Returns [`HortoError::Io`] on create failure in apply mode.
pub fn ensure_dir(ctx: &mut HostContext, path: &Path) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("mkdir -p {}", path.display()));
        return Ok(());
    }
    fs::create_dir_all(path)?;
    Ok(())
}

/// Write `content` to `path`, creating parent directories as needed.
///
/// In dry-run mode, records a planned write (byte length only) and returns.
///
/// # Errors
///
/// Returns [`HortoError::Io`] on parent create or write failure in apply mode.
pub fn write_file(ctx: &mut HostContext, path: &Path, content: &[u8]) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!(
            "write {} ({} bytes)",
            path.display(),
            content.len()
        ));
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

/// Copy a single file from `src` to `dest`, creating the dest parent as needed.
///
/// In dry-run mode, records a planned `cp` and returns.
///
/// # Errors
///
/// Returns [`HortoError::Message`] when the copy fails in apply mode (includes paths).
pub fn copy_file(ctx: &mut HostContext, src: &Path, dest: &Path) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("cp {} -> {}", src.display(), dest.display()));
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest).map_err(|e| {
        HortoError::msg(format!("copy {} -> {}: {e}", src.display(), dest.display()))
    })?;
    Ok(())
}

/// Recursively copy contents of `src` into `dest` (like `cp -a src/. dest/`).
///
/// In dry-run mode, records one planned action and returns without walking.
///
/// # Errors
///
/// Returns [`HortoError::Io`] on directory/file create or copy failure in apply mode.
pub fn copy_dir_contents(ctx: &mut HostContext, src: &Path, dest: &Path) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("cp -a {}/. -> {}/", src.display(), dest.display()));
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in WalkDir::new(src)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &target)?;
        }
    }
    Ok(())
}

/// Remove a file, symlink, or directory tree at `path`.
///
/// In dry-run mode, records a planned `rm -f` and returns.
///
/// # Errors
///
/// Returns [`HortoError::Io`] on remove failure in apply mode.
pub fn remove_path(ctx: &mut HostContext, path: &Path) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("rm -f {}", path.display()));
        return Ok(());
    }
    if path.is_symlink() || path.is_file() {
        fs::remove_file(path)?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Set Unix mode bits on `path` (`chmod`).
///
/// In dry-run mode, records a planned chmod and returns. On non-Unix apply builds,
/// the call is a no-op.
///
/// # Errors
///
/// Returns [`HortoError::Io`] when `set_permissions` fails on Unix apply mode.
pub fn chmod(ctx: &mut HostContext, path: &Path, mode: u32) -> Result<()> {
    if ctx.is_dry_run() {
        ctx.plan_action(format!("chmod {mode:o} {}", path.display()));
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}
