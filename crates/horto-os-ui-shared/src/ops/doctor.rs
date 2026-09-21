//! Local host readiness checks for status and pre-apply diagnostics.

use crate::context::{is_root, HostContext};
use crate::kits::docker;
use serde::{Deserialize, Serialize};

/// Boolean readiness probes (flattened in JSON).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorFlags {
    /// Process is effective uid 0.
    pub is_root: bool,
    /// `sudo` is on `PATH`.
    pub has_sudo: bool,
    /// `docker` is on `PATH`.
    pub docker_present: bool,
    /// [`HostPaths::active_setup`](crate::paths::HostPaths::active_setup) exists as a directory.
    pub active_setup_dir: bool,
    /// Full IoT-LAN env file is present.
    pub full_env: bool,
    /// Minimal setup env file is present.
    pub minimal_env: bool,
    /// Docker stacks directory exists.
    pub docker_dir: bool,
    /// Backup root directory exists.
    pub backup_dir: bool,
}

/// Boolean probes and free-form notes from [`doctor`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Flattened probe flags (same JSON field names as before nesting).
    #[serde(flatten)]
    pub flags: DoctorFlags,
    /// Operator-facing warnings (missing root, docker, env files, …).
    pub notes: Vec<String>,
}

impl std::ops::Deref for DoctorReport {
    type Target = DoctorFlags;

    fn deref(&self) -> &Self::Target {
        &self.flags
    }
}

/// Probe the host using `ctx.paths` and return a [`DoctorReport`].
#[must_use]
pub fn doctor(ctx: &HostContext) -> DoctorReport {
    let mut notes = Vec::new();
    let root = is_root();
    if !root {
        notes.push("Not running as root; apply mode needs sudo.".into());
    }
    let has_sudo = which::which("sudo").is_ok();
    let docker_present = docker::docker_available();
    if !docker_present {
        notes.push("docker binary not found on PATH.".into());
    }
    let active_setup_dir = ctx.paths.active_setup.is_dir();
    let full_env = ctx.paths.full_env_file().is_file();
    let minimal_env = ctx.paths.minimal_env_file().is_file();
    let os_conf = ctx.paths.os_configuration_file().is_file();
    let docker_dir = ctx.paths.docker.is_dir();
    let backup_dir = ctx.paths.backup.is_dir();
    if !full_env && !minimal_env && !os_conf {
        notes.push(
            "No active env yet (os-configuration.env, iot-lan_conf.env, or minimal_setup_vars.env)."
                .into(),
        );
    }
    DoctorReport {
        flags: DoctorFlags {
            is_root: root,
            has_sudo,
            docker_present,
            active_setup_dir,
            full_env,
            minimal_env,
            docker_dir,
            backup_dir,
        },
        notes,
    }
}
