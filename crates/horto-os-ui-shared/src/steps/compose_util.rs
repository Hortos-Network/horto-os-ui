//! Shared compose helpers for `d2` / `d3`.

use crate::context::HostContext;
use crate::error::{HortoError, Result};
use crate::kits::docker;
use std::path::Path;

/// True when `dir` has `compose.yaml` or `compose.yml`.
#[must_use]
pub fn compose_present(dir: &Path) -> bool {
    dir.join("compose.yaml").is_file() || dir.join("compose.yml").is_file()
}

/// Require compose files under `dir`.
///
/// # Errors
///
/// Returns [`HortoError`] when neither compose file exists.
pub fn ensure_compose(dir: &Path, name: &str) -> Result<()> {
    if compose_present(dir) {
        return Ok(());
    }
    Err(HortoError::msg(format!(
        "{name} compose missing under {}; run d1 first",
        dir.display()
    )))
}

/// Run `docker compose up -d` in `dir` (skipped under test hook).
///
/// # Errors
///
/// Returns [`HortoError`] when compose fails.
pub fn start_stack(ctx: &mut HostContext, name: &str, dir: &Path) -> Result<()> {
    ctx.log(format!("Starting {name} in {}", dir.display()));
    #[cfg(test)]
    {
        if SKIP_COMPOSE.with(std::cell::Cell::get) {
            ctx.log(format!("compose up skipped for {name} (test)"));
            return Ok(());
        }
    }
    docker::compose_up(dir)
}

/// True when a running container name contains `needle` (ASCII case-insensitive).
#[must_use]
pub fn container_name_contains(needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    docker::list_containers().is_ok_and(|rows| {
        rows.iter()
            .any(|c| c.names.to_ascii_lowercase().contains(&needle))
    })
}

#[cfg(test)]
thread_local! {
    /// When true, [`start_stack`] skips the real `docker compose` call.
    pub static SKIP_COMPOSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn compose_present_accepts_yml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("stack");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("compose.yml"), "services: {}\n").unwrap();
        assert!(compose_present(&dir));
        assert!(ensure_compose(&dir, "X").is_ok());
    }

    #[test]
    fn compose_present_accepts_yaml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("stack");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("compose.yaml"), "services: {}\n").unwrap();
        assert!(compose_present(&dir));
        assert!(!compose_present(tmp.path()));
    }

    #[test]
    fn ensure_compose_errors_when_absent() {
        let tmp = TempDir::new().unwrap();
        let err = ensure_compose(tmp.path(), "X").unwrap_err();
        assert!(err.to_string().contains("X compose missing"));
    }

    #[test]
    fn start_stack_respects_skip_compose_hook() {
        use crate::context::{ApplyMode, HostContext};
        use crate::paths::HostPaths;
        use crate::pipeline::SetupKind;

        SKIP_COMPOSE.with(|c| c.set(true));
        let tmp = TempDir::new().unwrap();
        let paths = HostPaths {
            active_setup: tmp.path().join("active"),
            backup: tmp.path().join("backup"),
            docker: tmp.path().join("docker"),
            etc: tmp.path().join("etc"),
            lease_file: tmp.path().join("leases"),
        };
        let dir = paths.docker.join("homepage");
        std::fs::create_dir_all(&dir).unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        start_stack(&mut ctx, "homepage", &dir).unwrap();
        assert!(ctx
            .logs
            .iter()
            .any(|l| l.contains("compose up skipped for homepage")));
        SKIP_COMPOSE.with(|c| c.set(false));
        let _ = container_name_contains("no-such-container-xyz");
    }
}
