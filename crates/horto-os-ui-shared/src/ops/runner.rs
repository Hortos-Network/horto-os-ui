//! Run Full/Minimal pipelines and individual steps with resume bookkeeping.

use crate::context::{require_root_for_apply, ApplyMode, HostContext};
use crate::error::{HortoError, Result};
use crate::pipeline::{self, SetupKind};
use crate::resume;
use crate::step::Step;

/// Execute every step in `kind`'s pipeline (dry-run or apply per `ctx.mode`).
///
/// # Errors
///
/// Returns [`HortoError::RootRequired`] in apply mode without root, or step / resume errors.
pub fn setup_run(ctx: &mut HostContext, kind: SetupKind) -> Result<()> {
    require_root_for_apply(ctx.mode)?;
    ctx.setup_kind = kind;
    for step in pipeline::pipeline(kind) {
        run_one(ctx, *step)?;
    }
    Ok(())
}

/// Execute a single step `id` if it belongs to `kind`'s pipeline.
///
/// # Errors
///
/// Returns [`HortoError::RootRequired`], [`HortoError::UnknownStep`],
/// [`HortoError::NotInPipeline`], [`HortoError::MissingDependency`], or step errors.
pub fn setup_step(ctx: &mut HostContext, kind: SetupKind, id: &str) -> Result<()> {
    require_root_for_apply(ctx.mode)?;
    ctx.setup_kind = kind;
    let step = pipeline::lookup(id).ok_or_else(|| HortoError::UnknownStep(id.into()))?;
    // Ensure the step is part of this kind's pipeline (d1 on full only, etc.)
    let in_pipe = pipeline::pipeline(kind).iter().any(|s| s.id() == id);
    if !in_pipe {
        return Err(HortoError::NotInPipeline {
            step: id.into(),
            kind: kind.as_str().into(),
        });
    }
    check_deps(ctx, step)?;
    run_one(ctx, step)
}

fn check_deps(ctx: &HostContext, step: &dyn Step) -> Result<()> {
    let state = resume::load(&ctx.paths.resume_file()).unwrap_or_default();
    let mut missing = Vec::new();
    for dep in step.depends_on() {
        let Some(dep_step) = pipeline::lookup(dep) else {
            continue;
        };
        let st = resume::status_for(&state, dep_step);
        let done = matches!(st, resume::StepStatus::Done) || dep_step.is_done(ctx);
        if !done {
            missing.push(*dep);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(HortoError::MissingDependency(
            step.id().into(),
            missing.join(", "),
        ))
    }
}

fn run_one(ctx: &mut HostContext, step: &dyn Step) -> Result<()> {
    ctx.log(format!("==> {} ({})", step.title(), step.id()));
    ctx.planned.clear();
    if ctx.mode == ApplyMode::DryRun {
        step.plan(ctx)?;
        // Also exercise apply path in dry-run so PlannedActions match real work.
        step.apply(ctx)?;
        return Ok(());
    }
    match step.apply(ctx) {
        Ok(()) => {
            resume::record_ok(&ctx.paths, step)?;
            if step.needs_reboot_after() {
                ctx.log("Note: a reboot is recommended after this step.");
            }
            Ok(())
        }
        Err(e) => {
            tracing::error!(step = step.id(), error = %e, "step failed");
            let _ = resume::mark_failed(&ctx.paths, step.id(), step.step_version());
            Err(e)
        }
    }
}
