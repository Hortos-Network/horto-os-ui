//! JSON-schema parameter structs for rmcp `Parameters<T>` tool handlers.

use schemars::JsonSchema;
use serde::Deserialize;

/// Confirm string for destructive HTTP mutate.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackupEtcArgs {
    /// Must be exactly `backup-etc`.
    pub confirm: String,
}

/// Apply mode and pipeline kind for setup tools.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetupRunArgs {
    /// When true, apply privileged changes on the box (default false = plan only).
    #[serde(default)]
    pub apply: bool,
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
    #[serde(default)]
    pub apply: bool,
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
