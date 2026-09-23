//! Remote apply path: PC shells out to OpenSSH and runs the CLI agent on a box.
//!
//! Default path never writes the box `authorized_keys`. Key install is explicit
//! opt-in via [`RemoteOptions::install_ssh_key`].

mod arch;
mod bins;
mod ecosystem;
mod host;
mod process;
mod runner;
mod ssh;
mod surfaces;
mod transfer;

#[cfg(test)]
use std::sync::Mutex;

/// Serialize tests that mutate process-global env (`XDG_CONFIG_HOME` / `HOME`).
#[cfg(test)]
pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

pub use arch::{box_arch_from_uname, BoxArch};
pub use bins::{
    asset_name, cache_bin_dir, default_cache_root, default_release_tag, ensure_local_bins,
    release_download_url, release_tag_is_immutable, resolve_local_ecosystem_bins, LocalBins,
    BOX_BIN_NAMES,
};
pub use ecosystem::{
    install_ecosystem_after_embedded_apply, install_ecosystem_services,
    remote_enable_ecosystem_script, unit_with_install_dir, EcosystemInstallChoice,
    API_TOKEN_DROP_BASENAME, ENSURE_API_TOKEN_SCRIPT, MCP_UNIT, STATUS_API_UNIT,
};
pub use host::{
    list_known_remote_hosts, merge_known_remote_hosts, parse_host_spec, parse_hosts_file_names,
    parse_ssh_config_host_aliases, parse_ssh_config_hosts, ssh_host_is_lan, HostSpec,
    KnownRemoteHost, SshConfigHost,
};
pub use process::{CommandOutput, ProcessRunner, StdioMode, SystemProcessRunner};
pub use runner::{
    classify_ssh_failure, finish_save_api_token, normalize_cli_version, offer_save_api_token,
    parse_api_token_drop, parse_remote_json, probe_remote_cli, pull_remote_api_token,
    remote_box_snapshot, remote_cli_version_is_current, remote_doctor, remote_doctor_report_banner,
    remote_ensure_ssh_key, remote_install_payload, remote_probe_arch, remote_progress_message,
    remote_reboot, remote_reboot_with_sudo_password, remote_run_banner_detail, remote_run_cli,
    remote_setup_run, remote_setup_status, remote_upload_cli, strip_tip_only_cli_flags,
    usable_api_token_hex, wants_reboot_now, write_api_token_file, RemoteBoxCliStatus,
    RemoteBoxSnapshot, RemoteCliProbe, RemoteOptions, RemoteOptionsInput, RemoteRunFlags,
    RemoteRunOutcome, RemoteRunRequest, RemoteSetupRunArgs, DEFAULT_GITHUB_REPO,
    DEFAULT_INSTALL_DIR, DEFAULT_REMOTE_AGENT_DIR, MIN_API_TOKEN_HEX_LEN,
};
pub use ssh::{remote_install_key_banner, SshSession};
pub use surfaces::{
    format_mcp_host_summary, format_surfaces_report, http_get_label, local_api_token_path,
    probe_api_surface, probe_cli_surface, probe_mcp_surface, probe_ssh_access, probe_ssh_surface,
    probe_surfaces, read_local_api_token, ApiSurfaceProbe, CliSurfaceProbe, McpHostProbe,
    SshSurfaceProbe, SurfaceProbeReport,
};
pub use transfer::{prefer_rsync, scp_files, transfer_files};
