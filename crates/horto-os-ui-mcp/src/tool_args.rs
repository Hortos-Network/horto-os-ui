//! JSON-schema parameter structs for rmcp `Parameters<T>` tool handlers.

use schemars::JsonSchema;
use serde::Deserialize;

/// Confirm string for destructive HTTP mutate.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackupEtcArgs {
    /// Must be exactly `backup-etc`.
    pub confirm: String,
}

/// Optional dry-run / pipeline kind for setup tools.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetupRunArgs {
    /// When true, plan only (default true).
    #[serde(default = "default_true")]
    pub dry_run: bool,
    /// Full pipeline when true (default true).
    #[serde(default = "default_true")]
    pub full: bool,
    /// Skip piper download on the box.
    #[serde(default)]
    pub skip_piper: bool,
}

/// Single setup step.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetupStepArgs {
    /// Step id (`s1`, `m1`, `d1`, …).
    pub step_id: String,
    #[serde(default = "default_true")]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub full: bool,
}

/// Confirm for docker rebuild.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DockerRebuildArgs {
    /// Must be exactly `docker-rebuild`.
    pub confirm: String,
}

fn default_true() -> bool {
    true
}
