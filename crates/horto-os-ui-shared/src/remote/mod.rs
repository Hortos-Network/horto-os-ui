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
mod transfer;

pub use arch::{box_arch_from_uname, BoxArch};
pub use bins::{
    asset_name, cache_bin_dir, default_cache_root, ensure_local_bins, release_download_url,
    LocalBins,
};
pub use host::{parse_host_spec, HostSpec};
pub use process::{CommandOutput, ProcessRunner, StdioMode, SystemProcessRunner};
pub use runner::{
    remote_install_payload, remote_probe_arch, remote_run_cli, remote_setup_run, RemoteOptions,
    RemoteRunRequest, DEFAULT_GITHUB_REPO, DEFAULT_INSTALL_DIR, DEFAULT_REMOTE_AGENT_DIR,
};
pub use ssh::{SshEnv, SshSession};
pub use transfer::{prefer_rsync, scp_files, transfer_files};
