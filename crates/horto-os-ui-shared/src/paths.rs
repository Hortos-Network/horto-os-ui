use std::path::{Path, PathBuf};

/// Default host layout used on a Horto box (`/srv/...`).
///
/// Tests and dry-run hosts override these with a temporary tree.
///
/// # Examples
///
/// ```
/// use horto_os_ui_shared::HostPaths;
/// use std::path::PathBuf;
///
/// let paths = HostPaths {
///     active_setup: PathBuf::from("/tmp/active"),
///     backup: PathBuf::from("/tmp/backup"),
///     docker: PathBuf::from("/tmp/docker"),
///     etc: PathBuf::from("/tmp/etc"),
///     lease_file: PathBuf::from("/tmp/leases"),
/// };
/// assert!(paths.resume_file().ends_with("horto_setup_state.json"));
/// ```
#[derive(Debug, Clone)]
pub struct HostPaths {
    /// `/srv/active_setup` (env files, staging, resume).
    pub active_setup: PathBuf,
    /// `/srv/backup`.
    pub backup: PathBuf,
    /// `/srv/docker`.
    pub docker: PathBuf,
    /// `/etc` (or a tempfile stand-in in tests).
    pub etc: PathBuf,
    /// dnsmasq leases file.
    pub lease_file: PathBuf,
}

impl Default for HostPaths {
    fn default() -> Self {
        Self {
            active_setup: PathBuf::from("/srv/active_setup"),
            backup: PathBuf::from("/srv/backup"),
            docker: PathBuf::from("/srv/docker"),
            etc: PathBuf::from("/etc"),
            lease_file: PathBuf::from("/var/lib/misc/dnsmasq.leases"),
        }
    }
}

impl HostPaths {
    /// Resume JSON path under `active_setup`.
    #[must_use]
    pub fn resume_file(&self) -> PathBuf {
        self.active_setup.join("horto_setup_state.json")
    }

    /// Full IoT-LAN env file (`my_variables.env`).
    #[must_use]
    pub fn full_env_file(&self) -> PathBuf {
        self.active_setup.join("my_variables.env")
    }

    /// Minimal setup env file.
    #[must_use]
    pub fn minimal_env_file(&self) -> PathBuf {
        self.active_setup.join("minimal_setup_vars.env")
    }

    /// OS configuration env under active setup.
    #[must_use]
    pub fn os_configuration_file(&self) -> PathBuf {
        self.active_setup.join("os-configuration.env")
    }

    /// Staged `/etc` tree before apply (`s4` → `s5`).
    #[must_use]
    pub fn staging_etc(&self) -> PathBuf {
        self.active_setup.join("etc")
    }

    /// Protected initial `/etc` backup destination (`s3`).
    #[must_use]
    pub fn initial_backup_etc(&self) -> PathBuf {
        self.backup.join("etc/initial_setup")
    }

    /// Docker assets directory (leases export, stack files).
    #[must_use]
    pub fn docker_assets(&self) -> PathBuf {
        self.docker.join("assets")
    }

    /// Exported leases JSON path.
    #[must_use]
    pub fn leases_json(&self) -> PathBuf {
        self.docker_assets().join("leases.json")
    }

    /// Exported leases HTML path.
    #[must_use]
    pub fn leases_html(&self) -> PathBuf {
        self.docker_assets().join("leases.html")
    }
}

/// Default `/srv/active_setup`.
pub const ACTIVE_SETUP: &str = "/srv/active_setup";
/// Default `/srv/backup`.
pub const BACKUP: &str = "/srv/backup";
/// Default `/srv/docker`.
pub const DOCKER: &str = "/srv/docker";
/// Default `/etc`.
pub const ETC: &str = "/etc";
/// Default resume file path on a Horto box.
pub const RESUME_FILE: &str = "/srv/active_setup/horto_setup_state.json";

/// Display helper for plan / log lines.
#[must_use]
pub fn path_str(p: &Path) -> String {
    p.display().to_string()
}
