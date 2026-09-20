//! Start Dockge and Homepage after d1 prepared `/srv/docker`.

use crate::context::{HostContext, PlannedAction};
use crate::error::{HortoError, Result};
use crate::kits::docker;
use crate::step::Step;
use crate::steps::d0_docker_engine::docker_engine_ready;

pub struct D2StartStacks;

impl Step for D2StartStacks {
    fn id(&self) -> &'static str {
        "d2"
    }
    fn title(&self) -> &'static str {
        "Start Dockge and Homepage"
    }
    fn reference_script(&self) -> &'static str {
        "d2_start_stacks.sh"
    }
    fn step_version(&self) -> u32 {
        1
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["d1"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        if !docker_engine_ready() {
            return false;
        }
        let dockge_dir = ctx.paths.docker.join("dockge");
        let homepage_dir = ctx.paths.docker.join("homepage");
        if !dockge_dir.join("compose.yaml").is_file() && !dockge_dir.join("compose.yml").is_file() {
            return false;
        }
        if !homepage_dir.join("compose.yaml").is_file()
            && !homepage_dir.join("compose.yml").is_file()
        {
            return false;
        }
        docker::list_containers().is_ok_and(|rows| {
            let names: String = rows
                .iter()
                .map(|c| c.names.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(",");
            names.contains("dockge") && names.contains("homepage")
        })
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let dockge = ctx.paths.docker.join("dockge");
        let homepage = ctx.paths.docker.join("homepage");
        ctx.plan_action(format!("docker compose up -d in {}", dockge.display()));
        ctx.plan_action(format!("docker compose up -d in {}", homepage.display()));
        ctx.plan_action("Dockge http://<hostname>:5001 ; Homepage http://<hostname>:3021");
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
        let dockge = ctx.paths.docker.join("dockge");
        let homepage = ctx.paths.docker.join("homepage");
        if !dockge.join("compose.yaml").is_file() && !dockge.join("compose.yml").is_file() {
            return Err(HortoError::msg(format!(
                "Dockge compose missing under {}; run d1 first",
                dockge.display()
            )));
        }
        if !homepage.join("compose.yaml").is_file() && !homepage.join("compose.yml").is_file() {
            return Err(HortoError::msg(format!(
                "Homepage compose missing under {}; run d1 first",
                homepage.display()
            )));
        }
        ctx.log(format!("Starting Dockge in {}", dockge.display()));
        docker::compose_up(&dockge)?;
        ctx.log(format!("Starting Homepage in {}", homepage.display()));
        docker::compose_up(&homepage)?;
        ctx.log(
            "Step d2 complete: Dockge http://<hostname>:5001 ; Homepage http://<hostname>:3021",
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApplyMode;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
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

    #[test]
    fn metadata_is_stable() {
        let step = D2StartStacks;
        assert_eq!(step.id(), "d2");
        assert_eq!(step.depends_on(), &["d1"]);
        assert_eq!(step.step_version(), 1);
    }

    #[test]
    fn dry_run_plans_compose_ups() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        D2StartStacks.apply(&mut ctx).unwrap();
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("dockge") && p.summary.contains("compose up")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("homepage") && p.summary.contains("compose up")));
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains(":5001") && p.summary.contains(":3021")));
    }
}
