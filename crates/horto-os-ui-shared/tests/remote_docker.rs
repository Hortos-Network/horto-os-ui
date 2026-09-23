//! Live OpenSSH practice against a local Docker "box".
//!
//! Ignored by default. Run:
//!
//! ```bash
//! cargo test -p horto-os-ui-shared --test remote_docker -- --ignored --nocapture
//! # or: make test-remote-docker
//! ```

#![cfg(unix)]

use horto_os_ui_shared::{
    remote_run_cli, EcosystemInstallChoice, RemoteOptions, RemoteRunFlags, RemoteRunRequest,
    SystemProcessRunner,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

const IMAGE: &str = "horto-os-ui-remote-ssh-test";
const CONTAINER: &str = "horto-os-ui-remote-ssh";

fn docker_ok() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote-ssh")
}

fn sh(args: &[&str]) -> std::process::Output {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", args[0]))
}

fn ensure_image() {
    let status = Command::new("docker")
        .args(["image", "inspect", IMAGE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("docker");
    if status.success() {
        return;
    }
    let dir = fixture_dir();
    let out = sh(&["docker", "build", "-t", IMAGE, dir.to_str().expect("utf8")]);
    assert!(
        out.status.success(),
        "docker build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn start_container(pub_key: &str) -> u16 {
    let _ = sh(&["docker", "rm", "-f", CONTAINER]);
    let out = sh(&[
        "docker",
        "run",
        "-d",
        "--name",
        CONTAINER,
        "-p",
        "127.0.0.1::22",
        IMAGE,
    ]);
    assert!(
        out.status.success(),
        "docker run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let inject = format!(
        "mkdir -p /home/horto/.ssh && echo '{}' > /home/horto/.ssh/authorized_keys && \
         chown -R horto:horto /home/horto/.ssh && chmod 600 /home/horto/.ssh/authorized_keys",
        pub_key.trim()
    );
    for _ in 0..40 {
        thread::sleep(Duration::from_millis(250));
        let ok = sh(&["docker", "exec", CONTAINER, "bash", "-lc", &inject]);
        if ok.status.success() {
            break;
        }
    }

    let port_out = sh(&["docker", "port", CONTAINER, "22/tcp"]);
    assert!(port_out.status.success());
    let mapped = String::from_utf8_lossy(&port_out.stdout);
    mapped
        .trim()
        .rsplit(':')
        .next()
        .expect("port")
        .parse::<u16>()
        .expect("parse port")
}

fn stop_container() {
    let _ = sh(&["docker", "rm", "-f", CONTAINER]);
}

#[test]
#[ignore = "needs Docker daemon; run make test-remote-docker"]
fn docker_ssh_remote_doctor_dry_path() {
    assert!(docker_ok(), "docker daemon not available");
    ensure_image();

    let tmp = TempDir::new().unwrap();
    let key = tmp.path().join("id_ed25519");
    let gen = sh(&[
        "ssh-keygen",
        "-t",
        "ed25519",
        "-N",
        "",
        "-f",
        key.to_str().unwrap(),
    ]);
    assert!(gen.status.success(), "ssh-keygen failed");

    let pub_key = fs::read_to_string(format!("{}.pub", key.display())).unwrap();
    let port = start_container(&pub_key);

    let fake_home = tmp.path().join("home");
    let ssh_dir = fake_home.join(".ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::copy(&key, ssh_dir.join("id_ed25519")).unwrap();
    fs::copy(
        format!("{}.pub", key.display()),
        ssh_dir.join("id_ed25519.pub"),
    )
    .unwrap();
    let cfg = format!(
        "Host horto-test-box\n\
         \tHostName 127.0.0.1\n\
         \tPort {port}\n\
         \tUser horto\n\
         \tIdentityFile {id}\n\
         \tIdentitiesOnly yes\n\
         \tStrictHostKeyChecking no\n\
         \tUserKnownHostsFile /dev/null\n",
        id = ssh_dir.join("id_ed25519").display()
    );
    fs::write(ssh_dir.join("config"), cfg).unwrap();
    let _ = sh(&["chmod", "700", ssh_dir.to_str().unwrap()]);
    let _ = sh(&["chmod", "600", ssh_dir.join("id_ed25519").to_str().unwrap()]);
    let _ = sh(&["chmod", "600", ssh_dir.join("config").to_str().unwrap()]);

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin_dir = tmp.path().join("bins");
    fs::create_dir_all(&bin_dir).unwrap();
    for name in [
        "horto-os-ui",
        "horto-os-ui-tui",
        "horto-os-ui-status-api",
        "horto-os-ui-mcp",
    ] {
        let src = workspace.join(format!("target/debug/{name}"));
        if src.is_file() {
            fs::copy(&src, bin_dir.join(name)).unwrap();
        } else {
            // Minimal stub so scp + chmod succeed; remote doctor needs a real CLI.
            fs::write(
                bin_dir.join(name),
                b"#!/bin/sh\necho '{\"ok\":true}'\nexit 0\n",
            )
            .unwrap();
            let _ = sh(&["chmod", "+x", bin_dir.join(name).to_str().unwrap()]);
        }
    }

    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &fake_home);

    let ssh_config = ssh_dir.join("config");
    let run = remote_run_cli(
        &SystemProcessRunner,
        &RemoteRunRequest {
            options: RemoteOptions {
                host: "horto-test-box".into(),
                install_ssh_key: false,
                bin_dir: Some(bin_dir),
                ssh_config_file: Some(ssh_config),
                ..RemoteOptions::default()
            },
            cli_args: vec!["doctor".into()],
            flags: RemoteRunFlags {
                use_sudo: false,
                install_payload_on_success: false,
                offer_reboot_on_success: false,
                capture_output: false,
                ..Default::default()
            },
            ecosystem: EcosystemInstallChoice::none(),
        },
    );

    if let Some(h) = old_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    stop_container();

    run.expect("remote_run_cli against Docker SSH box");
}
