//! High-level remote apply: probe arch, drop CLI agent, run setup, install payload.

use super::arch::{box_arch_from_uname, BoxArch};
use super::bins::{ensure_local_bins, LocalBins};
use super::host::parse_host_spec;
use super::process::{ProcessRunner, StdioMode};
use super::ssh::{SshEnv, SshSession};
use super::transfer::transfer_files;
use crate::error::Result;
use crate::VERSION;
use std::path::{Path, PathBuf};

/// Default GitHub repo that publishes box Release tar.gz assets.
pub const DEFAULT_GITHUB_REPO: &str = "Hortos-Network/horto-os-ui";

/// Remote directory for the temporary CLI apply agent.
pub const DEFAULT_REMOTE_AGENT_DIR: &str = "/tmp/horto-os-ui-remote";

/// Permanent install directory for CLI + TUI + API on the box.
pub const DEFAULT_INSTALL_DIR: &str = "/usr/local/bin";

/// Options for a remote OpenSSH session and binary source.
#[derive(Debug, Clone)]
pub struct RemoteOptions {
    /// OpenSSH Host alias or `user@host`.
    pub host: String,
    /// When true, run `ssh-copy-id` once before apply. Default false.
    pub install_ssh_key: bool,
    /// Local directory with the three box binaries (skips GitHub download).
    pub bin_dir: Option<PathBuf>,
    /// Workspace / Release version (`0.1.0`, without `v`). Used in asset filenames.
    pub version: String,
    /// GitHub Release tag to download from (`v0.1.0` or tip `dev-preview`).
    pub release_tag: String,
    /// `owner/repo` for Release downloads.
    pub github_repo: String,
    /// Force `SSH_ASKPASS` (Desktop / no TTY).
    pub force_askpass: bool,
    /// Cache root for downloaded Release assets.
    pub cache_root: PathBuf,
    /// Remote temp dir for the apply agent.
    pub remote_agent_dir: String,
    /// Remote install dir for permanent binaries.
    pub install_dir: String,
    /// Optional OpenSSH config file (`ssh -F`). Used by tests and custom layouts.
    pub ssh_config_file: Option<PathBuf>,
}

impl Default for RemoteOptions {
    fn default() -> Self {
        let version = VERSION.to_owned();
        let release_tag = super::bins::default_release_tag(&version);
        Self {
            host: String::new(),
            install_ssh_key: false,
            bin_dir: None,
            version,
            release_tag,
            github_repo: DEFAULT_GITHUB_REPO.to_owned(),
            force_askpass: false,
            cache_root: super::bins::default_cache_root(),
            remote_agent_dir: DEFAULT_REMOTE_AGENT_DIR.to_owned(),
            install_dir: DEFAULT_INSTALL_DIR.to_owned(),
            ssh_config_file: None,
        }
    }
}

/// CLI arguments to run on the box via the uploaded agent (without the binary name).
#[derive(Debug, Clone)]
pub struct RemoteRunRequest {
    /// Session and binary options.
    pub options: RemoteOptions,
    /// Arguments after the remote `horto-os-ui` binary (e.g. `["--dry-run", "setup", "run", "--full"]`).
    pub cli_args: Vec<String>,
    /// Prefix with `sudo -n` / `sudo` when true (apply mode).
    pub use_sudo: bool,
    /// After a successful command, install CLI+TUI+API and enable the API unit.
    pub install_payload_on_success: bool,
    /// After success (and payload install), offer an interactive box reboot (TTY only).
    pub offer_reboot_on_success: bool,
}

fn session_from(opts: &RemoteOptions) -> Result<SshSession> {
    Ok(SshSession {
        host: parse_host_spec(&opts.host)?,
        env: SshEnv {
            force_askpass: opts.force_askpass,
        },
        config_file: opts.ssh_config_file.clone(),
    })
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_owned();
    }
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | ':' | '+'))
    {
        return arg.to_owned();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn remote_agent_bin(opts: &RemoteOptions) -> String {
    format!(
        "{}/horto-os-ui",
        opts.remote_agent_dir.trim_end_matches('/')
    )
}

fn build_remote_command(opts: &RemoteOptions, cli_args: &[String], use_sudo: bool) -> String {
    let bin = remote_agent_bin(opts);
    let mut parts = Vec::new();
    if use_sudo {
        parts.push("sudo".to_owned());
    }
    parts.push(shell_quote(&bin));
    for a in cli_args {
        parts.push(shell_quote(a));
    }
    parts.join(" ")
}

/// Probe `uname -m` on the remote host.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or the arch is unsupported.
pub fn remote_probe_arch(runner: &dyn ProcessRunner, opts: &RemoteOptions) -> Result<BoxArch> {
    let session = session_from(opts)?;
    let out = session.exec(runner, "uname -m", StdioMode::Capture)?;
    box_arch_from_uname(&out.stdout)
}

fn ensure_agent(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
    bins: &LocalBins,
) -> Result<()> {
    session.exec(
        runner,
        &format!("mkdir -p {}", shell_quote(&opts.remote_agent_dir)),
        StdioMode::Capture,
    )?;
    let remote_bin = remote_agent_bin(opts);
    session.scp_to(runner, &bins.cli, &remote_bin)?;
    session.exec(
        runner,
        &format!("chmod +x {}", shell_quote(&remote_bin)),
        StdioMode::Capture,
    )?;
    Ok(())
}

fn maybe_install_key(
    runner: &dyn ProcessRunner,
    session: &SshSession,
    opts: &RemoteOptions,
) -> Result<()> {
    if opts.install_ssh_key {
        session.install_ssh_key(runner)?;
    }
    Ok(())
}

/// Upload the CLI agent (if needed), optionally install a public key, run a remote CLI command.
///
/// # Errors
///
/// Returns [`crate::HortoError`] on SSH/SCP/agent failures.
pub fn remote_run_cli(runner: &dyn ProcessRunner, req: &RemoteRunRequest) -> Result<String> {
    let opts = &req.options;
    let session = session_from(opts)?;
    let arch = {
        let out = session.exec(runner, "uname -m", StdioMode::Capture)?;
        box_arch_from_uname(&out.stdout)?
    };
    let bins = ensure_local_bins(
        runner,
        &opts.release_tag,
        &opts.version,
        &opts.github_repo,
        arch,
        opts.bin_dir.as_deref(),
        &opts.cache_root,
    )?;
    ensure_agent(runner, &session, opts, &bins)?;
    maybe_install_key(runner, &session, opts)?;

    let remote_cmd = build_remote_command(opts, &req.cli_args, req.use_sudo);
    // Inherit stdio so SSH/sudo password prompts work on a TTY (CLI/TUI).
    let out = session.exec(runner, &remote_cmd, StdioMode::Inherit)?;
    let mut log = out.stdout.clone();
    if !out.stderr.trim().is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&out.stderr);
    }
    if log.trim().is_empty() {
        log = format!("remote command finished: {remote_cmd}");
    }

    if req.install_payload_on_success {
        remote_install_payload(runner, opts, &bins)?;
    }
    if req.offer_reboot_on_success {
        offer_remote_reboot(runner, &session)?;
    }
    Ok(log)
}

/// Whether a reboot prompt answer means reboot now.
#[must_use]
pub fn wants_reboot_now(raw: &str) -> bool {
    matches!(raw.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Prompt on a local TTY, then `sudo reboot` on the box. No-op when stdin is not a TTY.
fn offer_remote_reboot(runner: &dyn ProcessRunner, session: &SshSession) -> Result<()> {
    use std::io::{self, IsTerminal, Write};

    if !io::stdin().is_terminal() {
        eprintln!("Reboot recommended for hostname/network changes. On the box: sudo reboot");
        return Ok(());
    }
    eprint!("Reboot the box now to apply hostname/network changes? [y/N]: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| crate::error::HortoError::msg(format!("read reboot prompt: {e}")))?;
    if !wants_reboot_now(&line) {
        eprintln!("Skipping reboot. Reboot the box later when convenient.");
        return Ok(());
    }
    eprintln!("Rebooting...");
    match session.exec(runner, "sudo reboot", StdioMode::Inherit) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Host drop mid-session is expected once reboot starts.
            eprintln!("reboot issued (SSH session closed is expected): {e}");
            Ok(())
        }
    }
}

/// Convenience: remote `setup run` with optional payload install.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when the remote setup fails.
pub fn remote_setup_run(
    runner: &dyn ProcessRunner,
    opts: RemoteOptions,
    dry_run: bool,
    full: bool,
    skip_piper: bool,
    install_payload: bool,
) -> Result<String> {
    let mut cli_args = Vec::new();
    if dry_run {
        cli_args.push("--dry-run".into());
    }
    if skip_piper {
        cli_args.push("--skip-piper".into());
    }
    cli_args.push("setup".into());
    cli_args.push("run".into());
    if full {
        cli_args.push("--full".into());
    } else {
        cli_args.push("--minimal".into());
    }
    remote_run_cli(
        runner,
        &RemoteRunRequest {
            options: opts,
            cli_args,
            use_sudo: !dry_run,
            install_payload_on_success: install_payload && !dry_run,
            offer_reboot_on_success: !dry_run,
        },
    )
}

const STATUS_API_UNIT: &str = r#"[Unit]
Description=Horto OS UI status API
After=network.target

[Service]
Type=simple
EnvironmentFile=/etc/horto-os-ui/api.env
ExecStart=/usr/local/bin/horto-os-ui-status-api --bind 0.0.0.0:8787
Restart=on-failure

[Install]
WantedBy=multi-user.target
"#;

/// Ensure `/etc/horto-os-ui/api.env` exists with a random bearer token (0600).
///
/// Reuses an existing file so reinstall does not rotate the token. Prints the
/// token once so the operator can paste it into Desktop Connection.
const ENSURE_API_TOKEN_SCRIPT: &str = r#"
set -e
sudo mkdir -p /etc/horto-os-ui
if [ ! -f /etc/horto-os-ui/api.env ]; then
  TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  printf 'HORTO_API_TOKEN=%s\n' "$TOKEN" | sudo tee /etc/horto-os-ui/api.env >/dev/null
  sudo chmod 600 /etc/horto-os-ui/api.env
fi
echo "Paste this bearer token into Desktop Connection (HORTO_API_TOKEN):"
sudo grep '^HORTO_API_TOKEN=' /etc/horto-os-ui/api.env
"#;

/// Copy CLI + TUI + API into `install_dir` and enable the status-api systemd unit.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when transfer or remote enable fails.
pub fn remote_install_payload(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    bins: &LocalBins,
) -> Result<()> {
    let session = session_from(opts)?;
    let staging = format!("{}/payload", opts.remote_agent_dir.trim_end_matches('/'));
    session.exec(
        runner,
        &format!("mkdir -p {}", shell_quote(&staging)),
        StdioMode::Capture,
    )?;

    let locals: [&Path; 3] = [&bins.cli, &bins.tui, &bins.status_api];
    transfer_files(runner, &session, &locals, &staging)?;

    let install = opts.install_dir.trim_end_matches('/');
    let unit_path = "/etc/systemd/system/horto-os-ui-status-api.service";
    session.exec(runner, ENSURE_API_TOKEN_SCRIPT, StdioMode::Inherit)?;
    let write_unit = format!(
        "sudo tee {unit_path} > /dev/null <<'HORTO_UNIT_EOF'\n{STATUS_API_UNIT}HORTO_UNIT_EOF"
    );
    let move_bins = format!(
        "sudo install -m 755 {staging}/horto-os-ui {staging}/horto-os-ui-tui {staging}/horto-os-ui-status-api {install}/ && \
         sudo systemctl daemon-reload && \
         sudo systemctl enable --now horto-os-ui-status-api.service"
    );
    session.exec(runner, &write_unit, StdioMode::Inherit)?;
    session.exec(runner, &move_bins, StdioMode::Inherit)?;

    if install != "/usr/local/bin" {
        let fix = format!(
            "sudo sed -i 's|/usr/local/bin/horto-os-ui-status-api|{install}/horto-os-ui-status-api|' {unit_path} && \
             sudo systemctl daemon-reload && sudo systemctl restart horto-os-ui-status-api.service"
        );
        session.exec(runner, &fix, StdioMode::Inherit)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::process::{CommandOutput, ScriptedRunner};
    use std::fs;
    use tempfile::TempDir;

    fn bin_dir_with_stubs() -> TempDir {
        let tmp = TempDir::new().unwrap();
        for name in ["horto-os-ui", "horto-os-ui-tui", "horto-os-ui-status-api"] {
            fs::write(tmp.path().join(name), b"#!/bin/true\n").unwrap();
        }
        tmp
    }

    #[test]
    fn shell_quote_safe_and_unsafe() {
        assert_eq!(shell_quote("setup"), "setup");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_remote_command_shapes() {
        let opts = RemoteOptions {
            remote_agent_dir: "/tmp/agent".into(),
            ..RemoteOptions::default()
        };
        let cmd = build_remote_command(
            &opts,
            &[
                "--dry-run".into(),
                "setup".into(),
                "run".into(),
                "--full".into(),
            ],
            false,
        );
        assert_eq!(cmd, "/tmp/agent/horto-os-ui --dry-run setup run --full");
        let cmd_sudo = build_remote_command(&opts, &["doctor".into()], true);
        assert!(cmd_sudo.starts_with("sudo "));
    }

    #[test]
    fn remote_run_does_not_copy_id_by_default() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        // uname
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        // mkdir agent
        runner.push("ssh", ScriptedRunner::ok(""));
        // scp agent
        runner.push("scp", ScriptedRunner::ok(""));
        // chmod
        runner.push("ssh", ScriptedRunner::ok(""));
        // remote cli
        runner.push("ssh", ScriptedRunner::ok("ok\n"));

        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    install_ssh_key: false,
                    ..RemoteOptions::default()
                },
                cli_args: vec!["--dry-run".into(), "setup".into(), "status".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
            },
        )
        .unwrap();
        assert!(log.contains("ok"));
        let programs: Vec<_> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _, _, _)| p.clone())
            .collect();
        assert!(!programs.iter().any(|p| p == "ssh-copy-id"));
    }

    #[test]
    fn remote_run_install_key_when_opt_in() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let stubs = bin_dir_with_stubs();
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::ok("aarch64\n"));
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: not authorized yet.
            runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
            runner.push("ssh-copy-id", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok("done\n"));

            remote_run_cli(
                &runner,
                &RemoteRunRequest {
                    options: RemoteOptions {
                        host: "box".into(),
                        bin_dir: Some(stubs.path().to_path_buf()),
                        install_ssh_key: true,
                        ..RemoteOptions::default()
                    },
                    cli_args: vec!["doctor".into()],
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                },
            )
            .unwrap();
            assert!(runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, _, _)| p == "ssh-copy-id"));
        });
    }

    #[test]
    fn remote_run_skips_ssh_copy_id_when_key_works() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let stubs = bin_dir_with_stubs();
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: already authorized.
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok("doctor ok\n"));

            remote_run_cli(
                &runner,
                &RemoteRunRequest {
                    options: RemoteOptions {
                        host: "box".into(),
                        bin_dir: Some(stubs.path().to_path_buf()),
                        install_ssh_key: true,
                        ..RemoteOptions::default()
                    },
                    cli_args: vec!["doctor".into()],
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                },
            )
            .unwrap();
            assert!(!runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _, _, _)| p == "ssh-copy-id"));
        });
    }

    #[test]
    fn default_options_key_off() {
        assert!(!RemoteOptions::default().install_ssh_key);
    }

    #[test]
    fn remote_probe_arch_ok() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("aarch64\n"));
        let arch = remote_probe_arch(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert_eq!(arch, BoxArch::Arm64);
    }

    #[test]
    fn remote_setup_run_dry_minimal_skip_piper() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("pipeline ok\n"));
        let log = remote_setup_run(
            &runner,
            RemoteOptions {
                host: "box".into(),
                bin_dir: Some(stubs.path().to_path_buf()),
                ..RemoteOptions::default()
            },
            true,
            false,
            true,
            false,
        )
        .unwrap();
        assert!(log.contains("pipeline ok") || log.contains("remote command"));
        let cli_ssh = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _, _, _)| p == "ssh")
            .unwrap()
            .1
            .join(" ");
        assert!(cli_ssh.contains("--dry-run"));
        assert!(cli_ssh.contains("--skip-piper"));
        assert!(cli_ssh.contains("--minimal"));
    }

    #[test]
    fn remote_run_merges_stderr_and_empty_inherit_log() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            CommandOutput {
                status: 0,
                stdout: "out\n".into(),
                stderr: "warn\n".into(),
            },
        );
        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
            },
        )
        .unwrap();
        assert!(log.contains("out"));
        assert!(log.contains("warn"));

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("scp", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        let log2 = remote_run_cli(
            &runner2,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
            },
        )
        .unwrap();
        assert!(log2.contains("remote command finished"));
    }

    #[test]
    fn remote_install_payload_scp_and_custom_prefix() {
        let stubs = bin_dir_with_stubs();
        let bins = LocalBins {
            dir: stubs.path().to_path_buf(),
            cli: stubs.path().join("horto-os-ui"),
            tui: stubs.path().join("horto-os-ui-tui"),
            status_api: stubs.path().join("horto-os-ui-status-api"),
        };
        let runner = ScriptedRunner::default();
        // mkdir staging
        runner.push("ssh", ScriptedRunner::ok(""));
        // prefer_rsync false
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        // scp_files mkdir + 3 scp
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // ensure api.env token + write unit + move
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        // custom install_dir sed
        runner.push("ssh", ScriptedRunner::ok(""));

        remote_install_payload(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                install_dir: "/opt/horto/bin".into(),
                ..RemoteOptions::default()
            },
            &bins,
        )
        .unwrap();
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "ssh")
                .count()
                >= 6
        );
    }

    #[test]
    fn remote_run_with_payload_install() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("done\n"));
        // payload
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // ensure api.env + write unit + move
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));

        remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["setup".into(), "run".into(), "--full".into()],
                use_sudo: true,
                install_payload_on_success: true,
                offer_reboot_on_success: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn wants_reboot_now_parses_answers() {
        assert!(wants_reboot_now("y"));
        assert!(wants_reboot_now("YES"));
        assert!(!wants_reboot_now(""));
        assert!(!wants_reboot_now("n"));
        assert!(!wants_reboot_now("maybe"));
    }
}
