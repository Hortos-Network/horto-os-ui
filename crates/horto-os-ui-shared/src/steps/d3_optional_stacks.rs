//! Optional Docker stacks after Homepage (`d3`).

use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::envfile;
use crate::ops::service_catalog::service_links_value;
use crate::step::Step;
use crate::steps::compose_util::{compose_present, ensure_compose, start_stack};
use crate::steps::d0_docker_engine::docker_engine_ready;
use std::collections::BTreeMap;
use std::path::Path;

/// Start opted Docker stacks and rewrite full `service_links.env` (`d3`).
pub struct D3OptionalStacks;

impl Step for D3OptionalStacks {
    fn id(&self) -> &'static str {
        "d3"
    }
    fn title(&self) -> &'static str {
        "Start optional Docker stacks"
    }
    fn reference_script(&self) -> &'static str {
        "d3_optional_stacks"
    }
    fn step_version(&self) -> u32 {
        2
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["d2"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        if !docker_engine_ready() {
            return false;
        }
        // Links file written after apply; do not require selected names in LINKS
        // (catalog is always full). Compose start is best-effort.
        ctx.paths.service_links_file().is_file()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        for stack in ctx.stack_opts.selected() {
            let dir = ctx.paths.docker.join(stack.dir);
            ctx.plan_action(format!(
                "docker compose up -d in {} ({})",
                dir.display(),
                stack.link
            ));
        }
        if !ctx.stack_opts.any() {
            ctx.plan_action("no optional Docker stacks selected");
        }
        ctx.plan_action(format!(
            "write service_links.env under {}",
            ctx.paths.active_setup.display()
        ));
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        if !docker_engine_ready() {
            return Err(HortoError::msg(
                "d3 requires Docker Engine; run d0 first (or install Docker)",
            ));
        }
        for stack in ctx.stack_opts.selected() {
            let dir = ctx.paths.docker.join(stack.dir);
            if !compose_present(&dir) {
                ctx.log(format!(
                    "warning: {} compose missing under {}; skipping",
                    stack.dir,
                    dir.display()
                ));
                continue;
            }
            if let Err(e) =
                ensure_compose(&dir, stack.dir).and_then(|()| start_stack(ctx, stack.dir, &dir))
            {
                ctx.log(format!(
                    "warning: failed to start {}: {e}; continuing",
                    stack.dir
                ));
            }
        }
        write_service_links(ctx)?;
        ctx.log("Step d3 complete: optional stacks and service_links updated.");
        Ok(())
    }
}

/// Build the full `LINKS=` value (every known service + default port).
#[must_use]
pub fn build_service_links_value() -> String {
    service_links_value()
}

/// Write `/srv/active_setup/service_links.env` with the full catalog.
///
/// # Errors
///
/// Returns filesystem errors.
pub fn write_service_links(ctx: &mut HostContext) -> Result<()> {
    let path = ctx.paths.service_links_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let links = build_service_links_value();
    let mut map = BTreeMap::new();
    map.insert("SCHEME".into(), "http".into());
    map.insert("HOST".into(), String::new());
    map.insert("LINKS".into(), links);
    if ctx.is_dry_run() {
        ctx.plan_action(format!("write {}", path.display()));
        return Ok(());
    }
    envfile::write(&path, &map)?;
    ctx.log(format!("Wrote {}", path.display()));
    Ok(())
}

/// Resolve compose path for tests / docs.
#[must_use]
pub fn stack_dir(docker_root: &Path, dir: &str) -> std::path::PathBuf {
    docker_root.join(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApplyMode;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
    use crate::remote::EcosystemInstallChoice;
    use crate::steps::compose_util::SKIP_COMPOSE;
    use crate::steps::d0_docker_engine::{reset_test_hooks, set_ready_override};
    use tempfile::TempDir;

    fn temp_paths(root: &std::path::Path) -> HostPaths {
        HostPaths {
            active_setup: root.join("active_setup"),
            backup: root.join("backup"),
            docker: root.join("docker"),
            etc: root.join("etc"),
            lease_file: root.join("leases"),
        }
    }

    fn write_compose(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("compose.yaml"), "services: {}\n").unwrap();
    }

    #[test]
    fn links_value_is_full_catalog() {
        let v = build_service_links_value();
        assert!(v.contains("Homepage:3021"));
        assert!(v.contains("Cockpit:9890"));
        assert!(v.contains("Dockge:5001"));
        assert!(v.contains("Status-API:8787"));
        assert!(v.contains("MCP:8790"));
        assert!(v.contains("Open-WebUI:3000"));
        assert!(v.contains("OpenWakeWord:10400"));
    }

    #[test]
    fn dry_run_plans_selected_stacks() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        ctx.stack_opts.dockge = true;
        ctx.stack_opts.open_webui = true;
        D3OptionalStacks.apply(&mut ctx).unwrap();
        assert!(ctx.planned.iter().any(|p| p.summary.contains("dockge")));
        assert!(ctx.planned.iter().any(|p| p.summary.contains("open-webui")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("service_links")));
    }

    #[test]
    fn dry_run_plans_none_selected_message() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        D3OptionalStacks.plan(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("no optional Docker stacks")));
    }

    #[test]
    fn is_done_false_without_engine_or_links() {
        reset_test_hooks();
        set_ready_override(Some(false));
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        assert!(!D3OptionalStacks.is_done(&ctx));
        set_ready_override(Some(true));
        assert!(!D3OptionalStacks.is_done(&ctx));
        reset_test_hooks();
    }

    #[test]
    fn apply_skips_missing_compose_and_writes_full_links() {
        reset_test_hooks();
        set_ready_override(Some(true));
        SKIP_COMPOSE.with(|c| c.set(true));
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        write_compose(&paths.docker.join("dockge"));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        ctx.stack_opts.dockge = true;
        ctx.stack_opts.whisper = true; // no compose → warn skip
        ctx.ecosystem = EcosystemInstallChoice::both();
        D3OptionalStacks.apply(&mut ctx).unwrap();
        assert!(ctx
            .logs
            .iter()
            .any(|l| l.contains("whisper-cv") && l.contains("skipping")));
        let links = envfile::load(&ctx.paths.service_links_file()).unwrap();
        let raw = links.get("LINKS").unwrap();
        assert!(raw.contains("Homepage:3021"));
        assert!(raw.contains("Dockge:5001"));
        assert!(raw.contains("Status-API:8787"));
        assert!(raw.contains("Whisper:8000"));
        assert!(raw.contains("Open-WebUI:3000"));
        SKIP_COMPOSE.with(|c| c.set(false));
        reset_test_hooks();
    }

    #[test]
    fn metadata_stable() {
        assert_eq!(D3OptionalStacks.id(), "d3");
        assert_eq!(D3OptionalStacks.depends_on(), &["d2"]);
    }
}
