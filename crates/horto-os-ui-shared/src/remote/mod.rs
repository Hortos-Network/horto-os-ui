//! Remote apply path: PC shells out to OpenSSH and runs the CLI agent on a box.
//!
//! Default path never writes the box `authorized_keys`. Key install is explicit
//! opt-in via [`RemoteOptions::install_ssh_key`].

mod arch;
mod bins;
mod host;
mod process;
mod runner;
mod ssh;
mod surfaces;
mod transfer;

pub use arch::{box_arch_from_uname, BoxArch};
pub use bins::{
    asset_name, cache_bin_dir, default_cache_root, default_release_tag, ensure_local_bins,
    release_download_url, release_tag_is_immutable, LocalBins,
};
pub use host::{parse_host_spec, HostSpec};
pub use process::{CommandOutput, ProcessRunner, StdioMode, SystemProcessRunner};
pub use runner::{
    classify_ssh_failure, finish_save_api_token, normalize_cli_version, offer_save_api_token,
    parse_api_token_drop, parse_remote_json, probe_remote_cli, remote_box_snapshot,
    remote_cli_version_is_current, remote_doctor, remote_doctor_report_banner,
    remote_ensure_ssh_key, remote_install_payload, remote_probe_arch, remote_progress_message,
    remote_reboot, remote_run_banner_detail, remote_run_cli, remote_setup_run, remote_setup_status,
    remote_upload_cli, wants_reboot_now, write_api_token_file, RemoteBoxCliStatus,
    RemoteBoxSnapshot, RemoteCliProbe, RemoteOptions, RemoteRunOutcome, RemoteRunRequest,
    DEFAULT_GITHUB_REPO, DEFAULT_INSTALL_DIR, DEFAULT_REMOTE_AGENT_DIR,
};
pub use ssh::{remote_install_key_banner, SshEnv, SshSession};
pub use surfaces::{
    format_surfaces_report, http_get_label, local_api_token_path, probe_ssh_access, probe_surfaces,
    read_local_api_token, ApiSurfaceProbe, CliSurfaceProbe, McpBoxProbe, McpPcProbe,
    SshSurfaceProbe, SurfaceProbeReport,
};
pub use transfer::{prefer_rsync, scp_files, transfer_files};
