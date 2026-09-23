//! Optional Docker stacks after Homepage (`d3`).

use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::envfile;
use crate::remote::EcosystemInstallChoice;
use crate::stack_opts::StackOpts;
use crate::step::Step;
use crate::steps::compose_util::{compose_present, ensure_compose, start_stack};
use crate::steps::d0_docker_engine::docker_engine_ready;
use std::collections::BTreeMap;
use std::path::Path;

/// Start opted Docker stacks and rewrite filtered `service_links.env` (`d3`).
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
        1
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["d2"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        if !docker_engine_ready() {
            return false;
        }
        let links = ctx.paths.service_links_file();
        if !links.is_file() {
            return false;
        }
        for stack in ctx.stack_opts.selected() {
            let dir = ctx.paths.docker.join(stack.dir);
            if compose_present(&dir) {
                // Opted stack with compose must have been started at least once;
                // presence of filtered links that include this stack is enough for resume.
                let Ok(map) = envfile::load(&links) else {
                    return false;
                };
                let Some(raw) = map.get("LINKS") else {
                    return false;
                };
                if !raw.to_ascii_lowercase().contains(
                    &stack
                        .link
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                ) {
                    return false;
                }
            }
        }
        true
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
            "write filtered service_links.env under {}",
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
        write_filtered_service_links(ctx)?;
        ctx.log("Step d3 complete: optional stacks and service_links updated.");
        Ok(())
    }
}

/// Build `LINKS=` value: Homepage + Cockpit always, then ecosystem and opted stacks.
#[must_use]
pub fn build_service_links_csv(stacks: StackOpts, eco: EcosystemInstallChoice) -> String {
    let mut parts = vec!["Homepage:3021".to_owned(), "Cockpit:9890".to_owned()];
    if eco.status_api {
        parts.push("Status-API:8787".into());
    }
    if eco.mcp {
        parts.push("MCP:8790".into());
    }
    for stack in stacks.selected() {
        parts.push(stack.link.to_owned());
    }
    parts.join(",")
}

/// Write `/srv/active_setup/service_links.env` for the current opts.
///
/// # Errors
///
/// Returns filesystem errors.
pub fn write_filtered_service_links(ctx: &mut HostContext) -> Result<()> {
    let path = ctx.paths.service_links_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let links = build_service_links_csv(ctx.stack_opts, ctx.ecosystem);
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
    fn links_csv_always_homepage_cockpit() {
        let csv = build_service_links_csv(StackOpts::none(), EcosystemInstallChoice::none());
        assert_eq!(csv, "Homepage:3021,Cockpit:9890");
    }

    #[test]
    fn links_csv_includes_ecosystem_and_stacks() {
        let stacks = StackOpts {
            dockge: true,
            evcc: true,
            ..StackOpts::none()
        };
        let csv = build_service_links_csv(stacks, EcosystemInstallChoice::both());
        assert!(csv.contains("Status-API:8787"));
        assert!(csv.contains("MCP:8790"));
        assert!(csv.contains("Dockge:5001"));
        assert!(csv.contains("EVCC:7070"));
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
    fn apply_skips_missing_compose_and_writes_links() {
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
        SKIP_COMPOSE.with(|c| c.set(false));
        reset_test_hooks();
    }

    #[test]
    fn metadata_stable() {
        assert_eq!(D3OptionalStacks.id(), "d3");
        assert_eq!(D3OptionalStacks.depends_on(), &["d2"]);
    }
}
