use crate::context::{is_root, HostContext};
use crate::kits::docker;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub is_root: bool,
    pub has_sudo: bool,
    pub docker_present: bool,
    pub active_setup_dir: bool,
    pub full_env: bool,
    pub minimal_env: bool,
    pub docker_dir: bool,
    pub backup_dir: bool,
    pub notes: Vec<String>,
}

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
        is_root: root,
        has_sudo,
        docker_present,
        active_setup_dir,
        full_env,
        minimal_env,
        docker_dir,
        backup_dir,
        notes,
    }
}
