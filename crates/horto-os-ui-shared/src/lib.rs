//! Horto box setup engine: modular versioned steps, kits, and ops.
//!
//! Surfaces (CLI, TUI, status-api) call this crate; they do not embed step logic.
//!
//! # Examples
//!
//! Dry-run the full pipeline against temporary host paths:
//!
//! ```
//! use horto_os_ui_shared::{setup_run, ApplyMode, HostContext, HostPaths, SetupKind};
//! use std::path::PathBuf;
//!
//! let root = PathBuf::from("/tmp/horto-doc-example");
//! let _ = std::fs::create_dir_all(root.join("active_setup"));
//! let paths = HostPaths {
//!     active_setup: root.join("active_setup"),
//!     backup: root.join("backup"),
//!     docker: root.join("docker"),
//!     etc: root.join("etc"),
//!     lease_file: root.join("dnsmasq.leases"),
//! };
//! let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
//! setup_run(&mut ctx, SetupKind::Full).expect("dry-run");
//! assert!(!ctx.logs.is_empty());
//! ```

pub mod build_info;
pub mod context;
pub mod embed;
pub mod error;
pub mod kits;
pub mod ops;
pub mod paths;
pub mod pipeline;
pub mod resume;
pub mod step;
pub mod steps;

pub use build_info::{footer_line, GIT_COMMIT, LONG_VERSION, VERSION};
pub use context::{
    require_root_for_apply, ApplyMode, HostContext, NonInteractivePrompts, PlannedAction,
    PromptsProvider, StdioPrompts,
};
pub use error::{HortoError, Result};
pub use ops::backup::{
    backup_disk, backup_etc_initial, backup_etc_timestamped, backup_shrink, backup_status,
    list_timestamped_etc_backups, plan_disk_backup, probe_disk_backup, BackupStatus,
    DiskBackupOpts, DiskBackupProbe, EtcBackupReport, ShrinkBackupOpts,
};
pub use ops::doctor::{doctor, DoctorReport};
pub use ops::leases::{export_dhcp_leases, read_leases, LeaseEntry};
pub use ops::runner::{setup_run, setup_step};
pub use ops::status::{box_status, setup_status, BoxStatus, SetupStatusReport};
pub use paths::HostPaths;
pub use pipeline::{lookup, pipeline, SetupKind};
pub use resume::{is_stale, StepStatus};
pub use step::Step;

pub use kits::docker::{
    docker_available, docker_rebuild, list_containers, parse_docker_ps_lines, ContainerInfo,
};
