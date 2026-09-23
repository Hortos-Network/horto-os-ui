//! Start Homepage after d1 prepared `/srv/docker` (`d2`).

use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::step::Step;
use crate::steps::compose_util::{
    compose_present, container_name_contains, ensure_compose, start_stack,
};
use crate::steps::d0_docker_engine::docker_engine_ready;

/// Start Homepage compose stack (`d2`). Dockge is optional in `d3`.
pub struct D2StartStacks;

impl Step for D2StartStacks {
    fn id(&self) -> &'static str {
        "d2"
    }
    fn title(&self) -> &'static str {
        "Start Homepage"
    }
    fn reference_script(&self) -> &'static str {
        "d2_start_stacks.sh"
    }
    fn step_version(&self) -> u32 {
        2
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["d1"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        if !docker_engine_ready() {
            return false;
        }
        if !compose_present(&ctx.paths.docker.join("homepage")) {
            return false;
        }
        container_name_contains("homepage")
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let homepage = ctx.paths.docker.join("homepage");
        ctx.plan_action(format!("docker compose up -d in {}", homepage.display()));
        ctx.plan_action("Homepage http://<hostname>:3021");
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        if ctx.is_dry_run() {
            self.plan(ctx)?;
            return Ok(());
        }
        if !docker_engine_ready() {
            return Err(HortoError::msg(
                "d2 requires Docker Engine; run d0 first (or install Docker)",
            ));
        }
        let homepage = ctx.paths.docker.join("homepage");
        ensure_compose(&homepage, "Homepage")?;
        start_stack(ctx, "Homepage", &homepage)?;
        ctx.log("Step d2 complete: Homepage http://<hostname>:3021");
        Ok(())
    }
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
    fn metadata_is_stable() {
        let step = D2StartStacks;
        assert_eq!(step.id(), "d2");
        assert_eq!(step.depends_on(), &["d1"]);
        assert_eq!(step.step_version(), 2);
        assert!(step.title().contains("Homepage"));
        assert_eq!(step.reference_script(), "d2_start_stacks.sh");
    }

    #[test]
    fn dry_run_plans_homepage_only() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        D2StartStacks.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("homepage") && p.summary.contains("compose up")));
        assert!(!ctx.planned.iter().any(|p| p.summary.contains("dockge")));
        assert!(ctx.planned.iter().any(|p| p.summary.contains(":3021")));
    }

    #[test]
    fn is_done_false_without_compose() {
        reset_test_hooks();
        set_ready_override(Some(true));
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        assert!(!D2StartStacks.is_done(&ctx));
        reset_test_hooks();
    }

    #[test]
    fn is_done_false_when_engine_missing() {
        reset_test_hooks();
        set_ready_override(Some(false));
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        write_compose(&paths.docker.join("homepage"));
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        assert!(!D2StartStacks.is_done(&ctx));
        reset_test_hooks();
    }

    #[test]
    fn apply_errors_without_engine() {
        reset_test_hooks();
        set_ready_override(Some(false));
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let err = D2StartStacks.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("requires Docker Engine"));
        reset_test_hooks();
    }

    #[test]
    fn apply_errors_when_homepage_compose_missing() {
        reset_test_hooks();
        set_ready_override(Some(true));
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let err = D2StartStacks.apply(&mut ctx).unwrap_err();
        assert!(err.to_string().contains("Homepage compose missing"));
        reset_test_hooks();
    }

    #[test]
    fn apply_starts_homepage_with_compose_skip() {
        reset_test_hooks();
        set_ready_override(Some(true));
        SKIP_COMPOSE.with(|c| c.set(true));
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        write_compose(&paths.docker.join("homepage"));
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        D2StartStacks.apply(&mut ctx).unwrap();
        assert!(ctx.logs.iter().any(|l| l.contains("Starting Homepage")));
        assert!(ctx.logs.iter().any(|l| l.contains("d2 complete")));
        assert!(!ctx.logs.iter().any(|l| l.contains("Starting Dockge")));
        SKIP_COMPOSE.with(|c| c.set(false));
        reset_test_hooks();
    }
}
