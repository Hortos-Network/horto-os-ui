//! reference: horto-os/scripts/s3_backup_etc_configs.sh
use crate::context::{HostContext, PlannedAction};
use crate::error::Result;
use crate::ops::backup;
use crate::step::Step;

pub struct S3Backup;

impl Step for S3Backup {
    fn id(&self) -> &'static str {
        "s3"
    }
    fn title(&self) -> &'static str {
        "Backup /etc paths listed in config"
    }
    fn reference_script(&self) -> &'static str {
        "s3_backup_etc_configs.sh"
    }
    fn step_version(&self) -> u32 {
        1
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        ctx.paths.initial_backup_etc().is_dir()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let dest = ctx.paths.initial_backup_etc();
        ctx.plan_action(format!("backup managed /etc entries -> {}", dest.display()));
        let _ = backup::copy_managed_etc(ctx, &dest)?;
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        backup::backup_etc_initial(ctx)?;
        Ok(())
    }
}
