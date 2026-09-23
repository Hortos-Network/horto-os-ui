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
pub mod remote;
pub mod resume;
pub mod stack_opts;
pub mod step;
pub mod steps;
pub mod tracing_init;

pub use build_info::{footer_line, GIT_COMMIT, LONG_VERSION, VERSION};
pub use context::{
    require_root_for_apply, ApplyMode, HostContext, NonInteractivePrompts, PlannedAction,
    PromptsProvider, StdioPrompts,
};
pub use error::{HortoError, Result};
pub use ops::backup::{
    backup_disk, backup_etc_initial, backup_etc_timestamped, backup_shrink, backup_status,
    list_timestamped_etc_backups, plan_disk_backup, probe_disk_backup, BackupStatus,
    DiskBackupFlags, DiskBackupOpts, DiskBackupProbe, EtcBackupReport, ShrinkBackupOpts,
};
pub use ops::catalog::{describe_container, describe_service};
pub use ops::doctor::{doctor, DoctorFlags, DoctorReport};
pub use ops::host_metrics::{collect_host_metrics, HostMetrics};
pub use ops::leases::{export_dhcp_leases, read_leases, LeaseEntry};
pub use ops::runner::{setup_run, setup_step};
pub use ops::status::{
    box_status, service_urls, setup_status, BoxStatus, SetupStatusReport, UrlInfo,
};
pub use paths::HostPaths;
pub use pipeline::{lookup, pipeline, SetupKind};
pub use remote::{
    classify_ssh_failure, finish_save_api_token, format_mcp_host_summary, format_surfaces_report,
    http_get_label, install_ecosystem_after_embedded_apply, install_ecosystem_services,
    list_known_remote_hosts, local_api_token_path, merge_known_remote_hosts, normalize_cli_version,
    offer_save_api_token, parse_api_token_drop, parse_host_spec, parse_hosts_file_names,
    parse_remote_json, parse_ssh_config_host_aliases, parse_ssh_config_hosts, probe_api_surface,
    probe_cli_surface, probe_mcp_surface, probe_remote_cli, probe_ssh_access, probe_ssh_surface,
    probe_surfaces, pull_remote_api_token, read_local_api_token, remote_box_snapshot,
    remote_cli_version_is_current, remote_doctor, remote_doctor_report_banner,
    remote_ensure_ssh_key, remote_install_key_banner, remote_install_payload, remote_probe_arch,
    remote_progress_message, remote_reboot, remote_reboot_with_sudo_password,
    remote_run_banner_detail, remote_run_cli, remote_setup_run, remote_setup_status,
    remote_upload_cli, resolve_local_ecosystem_bins, ssh_host_is_lan, strip_tip_only_cli_flags,
    usable_api_token_hex, wants_reboot_now, write_api_token_file, ApiSurfaceProbe, BoxArch,
    CliSurfaceProbe, EcosystemInstallChoice, HostSpec, KnownRemoteHost, LocalBins, McpHostProbe,
    ProcessRunner, RemoteBoxCliStatus, RemoteBoxSnapshot, RemoteCliProbe, RemoteOptions,
    RemoteOptionsInput, RemoteRunFlags, RemoteRunOutcome, RemoteRunRequest, RemoteSetupRunArgs,
    SshConfigHost, SshSession, SshSurfaceProbe, SurfaceProbeReport, SystemProcessRunner,
    BOX_BIN_NAMES, DEFAULT_GITHUB_REPO, DEFAULT_INSTALL_DIR, DEFAULT_REMOTE_AGENT_DIR,
    MIN_API_TOKEN_HEX_LEN,
};
pub use resume::{is_stale, StepStatus};
pub use stack_opts::{StackLink, StackOpts};
pub use step::Step;
pub use tracing_init::init_tracing;

pub use kits::docker::{
    docker_available, docker_rebuild, list_containers, parse_docker_ps_lines, resolve_stack,
    ContainerInfo,
};
