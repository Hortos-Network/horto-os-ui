//! High-level remote apply: probe arch, drop CLI agent, run setup, install payload.

mod apply;
mod options;
mod probe;
mod reboot;
mod status;
mod token;

pub use apply::{
    remote_ensure_ssh_key, remote_install_payload, remote_run_cli, remote_setup_run,
    remote_upload_cli, strip_tip_only_cli_flags, RemoteRunOutcome, RemoteSetupRunArgs,
};
pub use options::{
    remote_doctor_report_banner, remote_progress_message, remote_run_banner_detail, session_from,
    RemoteOptions, RemoteOptionsInput, RemoteRunFlags, RemoteRunRequest, DEFAULT_GITHUB_REPO,
    DEFAULT_INSTALL_DIR, DEFAULT_REMOTE_AGENT_DIR,
};
pub use probe::{
    classify_ssh_failure, normalize_cli_version, probe_remote_cli, remote_box_snapshot,
    remote_cli_version_is_current, remote_probe_arch, RemoteBoxCliStatus, RemoteBoxSnapshot,
    RemoteCliProbe,
};
pub use reboot::{remote_reboot, remote_reboot_with_sudo_password, wants_reboot_now};
pub use status::{parse_remote_json, remote_doctor, remote_setup_status};
pub use token::api_token_config_path;
pub use token::{
    finish_save_api_token, offer_save_api_token, parse_api_token_drop, pull_remote_api_token,
    usable_api_token_hex, write_api_token_file, MIN_API_TOKEN_HEX_LEN,
};

#[cfg(test)]
mod tests {
    use super::super::arch::BoxArch;
    use super::super::bins::LocalBins;
    use super::super::ecosystem::EcosystemInstallChoice;
    use super::super::ssh::SshSession;
    use super::options::{build_remote_command_at, remote_progress, shell_quote};
    use super::probe::ssh_command_detail;
    use super::reboot::finish_remote_reboot;
    use super::status::parse_setup_status_text;
    use super::token::offer_save_api_token_with;
    use super::*;
    use crate::remote::process::{CommandOutput, ScriptedRunner};
    use crate::LONG_VERSION;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Env vars for config path are process-global; serialize tests that mutate them.
    use crate::remote::ENV_LOCK as CONFIG_ENV_LOCK;

    /// Restore `HOME` / `XDG_CONFIG_HOME` even if a test panics mid-write.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: callers hold CONFIG_ENV_LOCK for the duration of this guard.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn bin_dir_with_stubs() -> TempDir {
        let tmp = TempDir::new().unwrap();
        for name in [
            "horto-os-ui",
            "horto-os-ui-tui",
            "horto-os-ui-status-api",
            "horto-os-ui-mcp",
        ] {
            fs::write(tmp.path().join(name), b"#!/bin/true\n").unwrap();
        }
        tmp
    }

    #[test]
    fn shell_quote_safe_and_unsafe() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("setup"), "setup");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn remote_progress_message_labels_host_and_detail() {
        assert_eq!(
            remote_progress_message("horto", "probe arch (SSH; may ask password)"),
            "[horto remote] PC → box 'horto': probe arch (SSH; may ask password)"
        );
    }

    #[test]
    fn parse_setup_status_text_reads_cli_lines() {
        let raw = "\
Setup kind: full
  [  done] s1 - Install base packages (v1)
  [pending] s5 - Apply staged configs to /etc (v2) [destructive] [reboot]
";
        let report = parse_setup_status_text(raw).unwrap();
        assert_eq!(report.kind, "full");
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[0].id, "s1");
        assert_eq!(report.steps[0].status, "done");
        assert_eq!(report.steps[1].id, "s5");
        assert!(report.steps[1].destructive);
        assert!(report.steps[1].needs_reboot_after);
    }

    #[test]
    fn parse_setup_status_text_skips_noise_and_allows_title_without_version() {
        let raw = "\
noise before
Setup kind: minimal
[broken
[x] no dash here
  [  done] s2 - Title only
";
        let report = parse_setup_status_text(raw).unwrap();
        assert_eq!(report.kind, "minimal");
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].id, "s2");
        assert_eq!(report.steps[0].title, "Title only");
        assert_eq!(report.steps[0].step_version, 0);
    }

    #[test]
    fn parse_setup_status_text_errors_without_steps() {
        let err = parse_setup_status_text("Setup kind: full\njust noise\n").unwrap_err();
        assert!(err.to_string().contains("no step lines"));
    }

    #[test]
    fn parse_remote_json_skips_leading_noise() {
        let raw = "ssh warn\n{\"is_root\":false,\"has_sudo\":true,\"docker_present\":true,\"active_setup_dir\":true,\"full_env\":true,\"minimal_env\":false,\"docker_dir\":true,\"backup_dir\":true,\"notes\":[]}\n";
        let doc: crate::ops::doctor::DoctorReport = parse_remote_json(raw).unwrap();
        assert!(doc.has_sudo);
        assert!(doc.docker_present);
    }

    #[test]
    fn remote_run_banner_detail_marks_sudo() {
        assert!(remote_run_banner_detail("horto-os-ui doctor", false).contains("SSH;"));
        assert!(!remote_run_banner_detail("horto-os-ui doctor", false).contains("sudo"));
        assert!(remote_run_banner_detail("sudo horto-os-ui setup", true).contains("SSH + sudo"));
    }

    #[test]
    fn remote_progress_and_doctor_banner_emit() {
        remote_progress("box", "unit-test detail");
        remote_doctor_report_banner();
    }

    fn push_cli_probes_missing(runner: &ScriptedRunner) {
        // install_dir then agent_dir: test -x fails → ssh non-zero
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
    }

    fn push_cli_probe_current(runner: &ScriptedRunner) {
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));
    }

    #[test]
    fn strip_tip_only_cli_flags_drops_ecosystem_and_stacks() {
        let args = vec![
            "--apply".into(),
            "--install-status-api".into(),
            "--no-install-mcp".into(),
            "--stacks=dockge".into(),
            "setup".into(),
            "run".into(),
            "--full".into(),
        ];
        let stripped = strip_tip_only_cli_flags(&args);
        assert_eq!(
            stripped,
            vec![
                "--apply".to_owned(),
                "setup".to_owned(),
                "run".to_owned(),
                "--full".to_owned(),
            ]
        );
    }

    #[test]
    fn remote_run_refuses_stale_cli_unless_allowed() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        // Gate probe: stale version on install path; agent path missing.
        runner.push("ssh", ScriptedRunner::ok("0.0.0 (deadbeef)\n"));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));

        let err = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec![
                    "--no-install-status-api".into(),
                    "setup".into(),
                    "run".into(),
                    "--full".into(),
                ],
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    allow_stale_cli: false,
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("out of date"), "unexpected err: {msg}");
        assert!(msg.contains("deadbeef"), "unexpected err: {msg}");

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner2);
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("scp", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok("0.0.0 (deadbeef)\n"));
        runner2.push("ssh", ScriptedRunner::fail(1, "missing"));
        runner2.push("ssh", ScriptedRunner::ok("stale ok\n"));
        let log = remote_run_cli(
            &runner2,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec![
                    "--no-install-status-api".into(),
                    "setup".into(),
                    "run".into(),
                    "--full".into(),
                ],
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    allow_stale_cli: true,
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("stale ok"));
        let cmd = runner2
            .calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(p, _, _, _)| p == "ssh")
            .map(|(_, args, _, _)| args.join(" "))
            .unwrap_or_default();
        assert!(!cmd.contains("--no-install-status-api"));
        assert!(cmd.contains("setup run --full"));
    }

    #[test]
    fn normalize_and_match_cli_version() {
        assert_eq!(
            normalize_cli_version(&format!("  {LONG_VERSION} \n")),
            LONG_VERSION
        );
        assert!(remote_cli_version_is_current(LONG_VERSION));
        assert!(remote_cli_version_is_current(&format!(
            "horto-os-ui {LONG_VERSION}"
        )));
        assert!(!remote_cli_version_is_current("0.0.0 (deadbeef)"));
        assert!(!remote_cli_version_is_current(""));
    }

    #[test]
    fn classify_ssh_failure_auth_vs_unreachable() {
        assert_eq!(
            classify_ssh_failure("exit 255: Permission denied (publickey,password)"),
            RemoteBoxCliStatus::AuthFailed
        );
        assert_eq!(
            classify_ssh_failure("exit 255: Connection refused"),
            RemoteBoxCliStatus::Unreachable
        );
        assert_eq!(
            classify_ssh_failure("exit 255: Could not resolve hostname"),
            RemoteBoxCliStatus::Unreachable
        );
    }

    #[test]
    fn remote_box_cli_status_as_label_covers_all_variants() {
        assert_eq!(RemoteBoxCliStatus::Missing.as_label(), "missing");
        assert_eq!(RemoteBoxCliStatus::AuthFailed.as_label(), "auth failed");
        assert_eq!(RemoteBoxCliStatus::Unreachable.as_label(), "unreachable");
        assert_eq!(
            RemoteBoxCliStatus::Found("0.1.0 (abc)".into()).as_label(),
            "0.1.0 (abc)"
        );
    }

    #[test]
    fn ssh_command_detail_filters_non_ssh_errors() {
        assert!(ssh_command_detail(&crate::error::HortoError::msg("nope")).is_none());
        assert_eq!(
            ssh_command_detail(&crate::error::HortoError::command("ssh", "exit 255: x")),
            Some("exit 255: x")
        );
        assert_eq!(
            ssh_command_detail(&crate::error::HortoError::command("scp", "denied")),
            Some("denied")
        );
        assert!(ssh_command_detail(&crate::error::HortoError::command("tar", "x")).is_none());
    }

    #[test]
    fn probe_remote_cli_reports_unreachable_on_connection_refused() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection refused"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::Unreachable);
    }

    #[test]
    fn probe_remote_cli_reports_auth_failed() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::AuthFailed);
        assert!(!probe.current);
    }

    #[test]
    fn probe_remote_cli_reports_missing_when_binaries_absent() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert_eq!(probe.status, RemoteBoxCliStatus::Missing);
    }

    #[test]
    fn remote_box_snapshot_no_scp_when_cli_current() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":["ok"]}"#,
            ),
        );

        let snap = remote_box_snapshot(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert!(snap.cli_current);
        assert_eq!(snap.setup.as_ref().unwrap().steps[0].id, "s1");
        assert!(snap.doctor.as_ref().unwrap().has_sudo);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0,
            "status refresh must never SCP when CLI is current"
        );
    }

    #[test]
    fn remote_box_snapshot_stale_skips_status_without_scp() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);

        let snap = remote_box_snapshot(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert!(!snap.cli_current);
        assert!(snap.setup.is_none());
        assert!(snap.doctor.is_none());
        assert_eq!(snap.cli_status, RemoteBoxCliStatus::Missing);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_box_snapshot_falls_back_to_text_status() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("not json\n"));
        runner.push(
            "ssh",
            ScriptedRunner::ok("Setup kind: full\n  [  done] s1 - Install base packages (v1)\n"),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":false,"active_setup_dir":false,"full_env":false,"minimal_env":true,"docker_dir":false,"backup_dir":false,"notes":[]}"#,
            ),
        );

        let snap = remote_box_snapshot(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert_eq!(snap.setup.as_ref().unwrap().steps[0].id, "s1");
        assert!(!snap.doctor.as_ref().unwrap().docker_present);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_upload_cli_scps_once() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        runner.push("ssh", ScriptedRunner::ok(&format!("{LONG_VERSION}\n")));

        let probe = remote_upload_cli(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                bin_dir: Some(stubs.path().to_path_buf()),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert!(probe.current);
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            1
        );
    }

    #[test]
    fn probe_remote_cli_skips_empty_version_then_accepts_agent() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("\n"));
        push_cli_probe_current(&runner);
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert!(probe.current);
        assert!(probe.path.unwrap().contains("horto-os-ui"));
    }

    #[test]
    fn probe_remote_cli_keeps_stale_install_when_agent_missing() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("0.0.0 (deadbeef)\n"));
        runner.push("ssh", ScriptedRunner::fail(1, "missing"));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        let probe = probe_remote_cli(&runner, &session_from(&opts).unwrap(), &opts).unwrap();
        assert!(!probe.current);
        assert_eq!(probe.version.as_deref(), Some("0.0.0 (deadbeef)"));
        assert!(probe.path.is_some());
    }

    #[test]
    fn remote_run_skips_scp_when_box_cli_already_current() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probe_current(&runner);
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("ok\n"));

        let log = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
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
        )
        .unwrap()
        .log;
        assert!(log.contains("ok"));
        assert_eq!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _, _, _)| p == "scp")
                .count(),
            0
        );
    }

    #[test]
    fn remote_run_capture_output_merges_stderr() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probe_current(&runner);
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            CommandOutput {
                status: 0,
                stdout: "out-line\n".into(),
                stderr: "warn-line\n".into(),
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
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: true,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("out-line"));
        assert!(log.contains("warn-line"));
    }

    #[test]
    fn remote_setup_status_and_doctor_require_current_cli() {
        let runner = ScriptedRunner::default();
        push_cli_probes_missing(&runner);
        let err = remote_setup_status(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("sync CLI"));

        let runner2 = ScriptedRunner::default();
        push_cli_probes_missing(&runner2);
        let err = remote_doctor(
            &runner2,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("sync CLI"));
    }

    #[test]
    fn remote_setup_status_and_doctor_return_reports_when_current() {
        let runner = ScriptedRunner::default();
        push_cli_probe_current(&runner);
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":[]}"#,
            ),
        );
        let status = remote_setup_status(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            true,
        )
        .unwrap();
        assert_eq!(status.steps[0].id, "s1");

        let runner2 = ScriptedRunner::default();
        push_cli_probe_current(&runner2);
        runner2.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"kind":"full","steps":[{"id":"s1","title":"Base","status":"done","step_version":1,"destructive":false,"needs_reboot_after":false}]}"#,
            ),
        );
        runner2.push(
            "ssh",
            ScriptedRunner::ok(
                r#"{"is_root":false,"has_sudo":true,"docker_present":true,"active_setup_dir":true,"full_env":true,"minimal_env":false,"docker_dir":true,"backup_dir":true,"notes":[]}"#,
            ),
        );
        let doc = remote_doctor(
            &runner2,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert!(doc.has_sudo);
    }

    #[test]
    fn build_remote_command_shapes() {
        let cmd = build_remote_command_at(
            "/tmp/agent/horto-os-ui",
            &[
                "--apply".into(),
                "setup".into(),
                "run".into(),
                "--full".into(),
            ],
            false,
        );
        assert_eq!(cmd, "/tmp/agent/horto-os-ui --apply setup run --full");
        let cmd_sudo = build_remote_command_at("/tmp/agent/horto-os-ui", &["doctor".into()], true);
        assert!(cmd_sudo.starts_with("sudo "));
    }

    #[test]
    fn remote_run_does_not_copy_id_by_default() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        // uname
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        // mkdir agent
        runner.push("ssh", ScriptedRunner::ok(""));
        // scp agent
        runner.push("scp", ScriptedRunner::ok(""));
        // chmod
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
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
                cli_args: vec!["setup".into(), "status".into()],
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap()
        .log;
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
            push_cli_probes_missing(&runner);
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: not authorized yet.
            runner.push("ssh", ScriptedRunner::fail(255, "Permission denied"));
            runner.push("ssh-copy-id", ScriptedRunner::ok(""));
            push_cli_probe_current(&runner);
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
                    flags: RemoteRunFlags {
                        use_sudo: false,
                        install_payload_on_success: false,
                        offer_reboot_on_success: false,
                        capture_output: false,
                        ..Default::default()
                    },
                    ecosystem: EcosystemInstallChoice::none(),
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
            push_cli_probes_missing(&runner);
            runner.push("ssh", ScriptedRunner::ok(""));
            runner.push("scp", ScriptedRunner::ok(""));
            runner.push("ssh", ScriptedRunner::ok(""));
            // Key probe: already authorized.
            runner.push("ssh", ScriptedRunner::ok(""));
            push_cli_probe_current(&runner);
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
                    flags: RemoteRunFlags {
                        use_sudo: false,
                        install_payload_on_success: false,
                        offer_reboot_on_success: false,
                        capture_output: false,
                        ..Default::default()
                    },
                    ecosystem: EcosystemInstallChoice::none(),
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
    fn remote_options_from_input_tag_and_defaults() {
        let opts = RemoteOptions::from_input(RemoteOptionsInput {
            host: "box".into(),
            install_ssh_key: true,
            bin_dir: Some(PathBuf::from("/bins")),
            release_tag: Some("dev-preview".into()),
        });
        assert_eq!(opts.host, "box");
        assert!(opts.install_ssh_key);
        assert_eq!(opts.bin_dir.as_deref(), Some(Path::new("/bins")));
        assert_eq!(opts.release_tag, "dev-preview");
        let defaults = RemoteOptions::default();
        assert_eq!(defaults.host, "");
        assert!(!defaults.install_ssh_key);
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
    fn remote_setup_run_plan_minimal_skip_piper() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("pipeline ok\n"));
        let log = remote_setup_run(
            &runner,
            RemoteSetupRunArgs {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                apply: false,
                full: false,
                skip_piper: true,
                ecosystem: EcosystemInstallChoice::none(),
                stack_opts: crate::stack_opts::StackOpts::none(),
                allow_stale_cli: false,
            },
        )
        .unwrap()
        .log;
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
        assert!(!cli_ssh.contains("--apply"));
        assert!(cli_ssh.contains("--skip-piper"));
        assert!(cli_ssh.contains("--minimal"));
    }

    #[test]
    fn remote_run_merges_stderr_and_empty_inherit_log() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
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
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap()
        .log;
        assert!(log.contains("out"));
        assert!(log.contains("warn"));

        let runner2 = ScriptedRunner::default();
        runner2.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner2);
        runner2.push("ssh", ScriptedRunner::ok(""));
        runner2.push("scp", ScriptedRunner::ok(""));
        runner2.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner2);
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
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap()
        .log;
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
            mcp: Some(stubs.path().join("horto-os-ui-mcp")),
        };
        let runner = ScriptedRunner::default();
        // mkdir staging
        runner.push("ssh", ScriptedRunner::ok(""));
        // prefer_rsync false
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        // scp_files mkdir + 4 scp
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // one enable (token+units+bins[+restart]) + cat drop + rm drop
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            ScriptedRunner::ok("HORTO_API_TOKEN=deadbeefcafebabedeadbeefcafebabe\n"),
        );
        runner.push("ssh", ScriptedRunner::ok(""));

        let token = remote_install_payload(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                install_dir: "/opt/horto/bin".into(),
                ..RemoteOptions::default()
            },
            &bins,
            EcosystemInstallChoice {
                status_api: true,
                mcp: true,
            },
        )
        .unwrap();
        assert_eq!(token.as_deref(), Some("deadbeefcafebabedeadbeefcafebabe"));
        let ssh_cmds: Vec<String> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _, _)| p == "ssh")
            .map(|(_, args, _, _)| args.join(" "))
            .collect();
        assert!(ssh_cmds
            .iter()
            .any(|c| c.contains("horto-os-ui-status-api.service")
                && c.contains("horto-os-ui-mcp.service")
                && c.contains("/opt/horto/bin")
                && c.contains("restart horto-os-ui-mcp.service")));
        assert!(ssh_cmds.len() >= 5);
    }

    #[test]
    fn remote_run_with_payload_install() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("done\n"));
        // payload
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // enable (token+units+bins) + cat drop + rm
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            ScriptedRunner::ok("aabbccddeeff00112233445566778899\n"),
        );
        runner.push("ssh", ScriptedRunner::ok(""));

        let outcome = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["setup".into(), "run".into(), "--full".into()],
                flags: RemoteRunFlags {
                    use_sudo: true,
                    install_payload_on_success: true,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice {
                    status_api: true,
                    mcp: true,
                },
            },
        )
        .unwrap();
        assert!(outcome.log.contains("done"));
        assert_eq!(
            outcome.api_token.as_deref(),
            Some("aabbccddeeff00112233445566778899")
        );
    }

    #[test]
    fn remote_run_payload_skipped_when_ecosystem_none() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("done\n"));

        let outcome = remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["setup".into(), "run".into(), "--full".into()],
                flags: RemoteRunFlags {
                    use_sudo: true,
                    install_payload_on_success: true,
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap();
        assert!(outcome.log.contains("done"));
        assert!(outcome.api_token.is_none());
        let scp = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _, _)| p == "scp" || p == "rsync")
            .count();
        // Only CLI upload SCP, no payload transfer of four bins.
        assert_eq!(scp, 1);
    }

    #[test]
    fn parse_api_token_drop_accepts_prefix_and_bare_hex() {
        assert_eq!(
            parse_api_token_drop("HORTO_API_TOKEN=aabbccddeeff00112233445566778899\n").as_deref(),
            Some("aabbccddeeff00112233445566778899")
        );
        assert_eq!(
            parse_api_token_drop("  deadbeefcafebabedeadbeefcafebabe  \n").as_deref(),
            Some("deadbeefcafebabedeadbeefcafebabe")
        );
        assert!(parse_api_token_drop("").is_none());
        assert!(parse_api_token_drop("not-hex!").is_none());
        assert!(parse_api_token_drop("HORTO_API_TOKEN=abc123\n").is_none());
        assert!(parse_api_token_drop("11223344\n").is_none());
    }

    #[test]
    fn finish_save_api_token_writes_under_xdg_config() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        assert!(!finish_save_api_token("aabb", "n").unwrap());
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        assert!(finish_save_api_token("aabbccddeeff00112233445566778899", "y").unwrap());
        let path = tmp.path().join("horto-os-ui").join("api_token");
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.trim(), "aabbccddeeff00112233445566778899");
    }

    #[test]
    fn write_api_token_file_uses_home_when_xdg_empty() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", "   ");
        let _home = EnvVarGuard::set("HOME", tmp.path());
        let path = write_api_token_file("11223344556677889900aabbccddeeff").unwrap();
        assert_eq!(
            path,
            tmp.path()
                .join(".config")
                .join("horto-os-ui")
                .join("api_token")
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim(),
            "11223344556677889900aabbccddeeff"
        );
    }

    #[test]
    fn write_api_token_file_uses_home_when_xdg_unset() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::remove("XDG_CONFIG_HOME");
        let _home = EnvVarGuard::set("HOME", tmp.path());
        let path = write_api_token_file("556677889900aabbccddeeff11223344").unwrap();
        assert!(path.ends_with("horto-os-ui/api_token"));
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim(),
            "556677889900aabbccddeeff11223344"
        );
    }

    #[test]
    fn offer_save_api_token_non_tty_skips_write() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        assert!(
            !offer_save_api_token_with("99aabbccddeeff001122334455667788", false, None).unwrap()
        );
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        // Public wrapper still exercises is_terminal() + dispatch.
        let _ = offer_save_api_token("99aabbccddeeff001122334455667788");
    }

    #[test]
    fn offer_save_api_token_tty_canned_yes_and_no() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        assert!(!offer_save_api_token_with("aa11", true, Some("n")).unwrap());
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
        assert!(
            offer_save_api_token_with("bb22cc33ddeeff001122334455667788", true, Some("yes"))
                .unwrap()
        );
        let body = fs::read_to_string(tmp.path().join("horto-os-ui").join("api_token")).unwrap();
        assert_eq!(body.trim(), "bb22cc33ddeeff001122334455667788");
    }

    #[test]
    fn parse_api_token_drop_rejects_empty_hex_value() {
        assert!(parse_api_token_drop("HORTO_API_TOKEN=\n").is_none());
        assert!(parse_api_token_drop("HORTO_API_TOKEN= \n").is_none());
        assert_eq!(
            parse_api_token_drop("\n\nHORTO_API_TOKEN=aabbccddeeff00112233445566778899\n")
                .as_deref(),
            Some("aabbccddeeff00112233445566778899")
        );
    }

    #[test]
    fn write_api_token_file_rejects_short_before_create() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        let err = write_api_token_file("11223344").unwrap_err();
        assert!(err.to_string().contains("at least"));
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
    }

    #[test]
    fn pull_remote_api_token_rejects_empty_host() {
        let runner = ScriptedRunner::default();
        let err = pull_remote_api_token(
            &runner,
            &RemoteOptions {
                host: "  ".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn pull_remote_api_token_writes_tip_from_drop() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        let runner = ScriptedRunner::default();
        // Inherit sudo grep into drop file.
        runner.push("ssh", ScriptedRunner::ok(""));
        // Capture cat of drop file.
        runner.push(
            "ssh",
            ScriptedRunner::ok("HORTO_API_TOKEN=aabbccddeeff00112233445566778899\n"),
        );
        let tok = pull_remote_api_token(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap();
        assert_eq!(tok, "aabbccddeeff00112233445566778899");
        let tip = tmp.path().join("horto-os-ui").join("api_token");
        assert_eq!(fs::read_to_string(&tip).unwrap().trim(), tok);
    }

    #[test]
    fn pull_remote_api_token_errors_when_drop_unusable() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", tmp.path());
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("HORTO_API_TOKEN=short\n"));
        let err = pull_remote_api_token(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("could not read Status API token"));
        assert!(!tmp.path().join("horto-os-ui").join("api_token").exists());
    }

    #[test]
    fn write_api_token_file_errors_when_config_parent_blocked() {
        let _guard = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("blocked");
        fs::write(&blocker, b"not-a-directory").unwrap();
        let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", &blocker);
        let err = write_api_token_file("aabbccddeeff00112233445566778899").unwrap_err();
        assert!(err.to_string().contains("create"));
    }

    #[test]
    fn remote_install_payload_default_prefix_none_token() {
        let stubs = bin_dir_with_stubs();
        let bins = LocalBins {
            dir: stubs.path().to_path_buf(),
            cli: stubs.path().join("horto-os-ui"),
            tui: stubs.path().join("horto-os-ui-tui"),
            status_api: stubs.path().join("horto-os-ui-status-api"),
            mcp: Some(stubs.path().join("horto-os-ui-mcp")),
        };
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        // enable (token+units+bins) + empty drop + rm (default /usr/local/bin: no restart)
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok("\n"));
        runner.push("ssh", ScriptedRunner::ok(""));

        let token = remote_install_payload(
            &runner,
            &RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            },
            &bins,
            EcosystemInstallChoice {
                status_api: true,
                mcp: true,
            },
        )
        .unwrap();
        assert!(token.is_none());
    }

    #[test]
    fn remote_setup_run_full_apply_captures_token() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("setup ok\n"));
        // payload
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("rsync", ScriptedRunner::fail(127, "no"));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push(
            "ssh",
            ScriptedRunner::ok("HORTO_API_TOKEN=ffeeddccbbaa99887766554433221100\n"),
        );
        runner.push("ssh", ScriptedRunner::ok(""));

        let outcome = remote_setup_run(
            &runner,
            RemoteSetupRunArgs {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                apply: true,
                full: true,
                skip_piper: false,
                ecosystem: EcosystemInstallChoice {
                    status_api: true,
                    mcp: true,
                },
                stack_opts: crate::stack_opts::StackOpts::none(),
                allow_stale_cli: false,
            },
        )
        .unwrap();
        assert!(outcome.log.contains("setup ok"));
        assert_eq!(
            outcome.api_token.as_deref(),
            Some("ffeeddccbbaa99887766554433221100")
        );
        let cli_ssh = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|(p, args, _, _)| p == "ssh" && args.iter().any(|a| a.contains("--full")))
            .map(|(_, args, _, _)| args.join(" "))
            .unwrap_or_default();
        assert!(cli_ssh.contains("--full"));
        assert!(cli_ssh.contains("--apply"));
    }

    #[test]
    fn remote_run_outcome_default_is_empty() {
        let o = RemoteRunOutcome::default();
        assert_eq!(o.log, "");
        assert!(o.api_token.is_none());
    }

    #[test]
    fn wants_reboot_now_parses_answers() {
        assert!(wants_reboot_now("y"));
        assert!(wants_reboot_now("YES"));
        assert!(!wants_reboot_now(""));
        assert!(!wants_reboot_now("n"));
        assert!(!wants_reboot_now("maybe"));
    }

    #[test]
    fn ssh_drop_after_reboot_matches_expected_phrases() {
        for msg in [
            "Connection closed by remote host",
            "Connection reset by peer",
            "Broken pipe",
            "ssh: exit 255",
        ] {
            assert!(
                super::reboot::ssh_drop_after_reboot(&crate::error::HortoError::msg(msg)),
                "expected drop: {msg}"
            );
        }
        assert!(!super::reboot::ssh_drop_after_reboot(
            &crate::error::HortoError::msg("Sorry, try again.")
        ));
    }

    fn test_session() -> SshSession {
        session_from(&RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        })
        .unwrap()
    }

    #[test]
    fn finish_reboot_no_skips_ssh() {
        let runner = ScriptedRunner::default();
        finish_remote_reboot(&runner, &test_session(), "n", None).unwrap();
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn finish_reboot_yes_runs_sudo_reboot() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        finish_remote_reboot(&runner, &test_session(), "yes", None).unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.iter().any(|a| a.contains("sudo reboot")));
        drop(calls);
    }

    #[test]
    fn finish_reboot_yes_treats_ssh_drop_as_ok() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(255, "Connection closed"));
        finish_remote_reboot(&runner, &test_session(), "y", None).unwrap();
    }

    #[test]
    fn finish_reboot_with_password_feeds_sudo_dash_s() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        finish_remote_reboot(&runner, &test_session(), "y", Some("pw")).unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.iter().any(|a| a.contains("sudo -S reboot")));
        assert!(!calls[0].1.iter().any(|a| *a == "-tt"));
        drop(calls);
    }

    #[test]
    fn finish_reboot_with_password_propagates_sudo_failure() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::fail(1, "Sorry, try again."));
        let err = finish_remote_reboot(&runner, &test_session(), "y", Some("bad")).unwrap_err();
        assert!(err.to_string().contains("Sorry") || err.to_string().contains("exit 1"));
    }

    #[test]
    fn finish_reboot_with_password_treats_ssh_drop_as_ok() {
        for stderr in [
            "Connection closed by remote host",
            "Connection reset by peer",
            "Broken pipe",
            "ssh: exit 255",
        ] {
            let runner = ScriptedRunner::default();
            runner.push("ssh", ScriptedRunner::fail(255, stderr));
            finish_remote_reboot(&runner, &test_session(), "y", Some("pw")).unwrap();
        }
    }

    #[test]
    fn remote_reboot_with_sudo_password_wrapper() {
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok(""));
        let opts = RemoteOptions {
            host: "box".into(),
            ..RemoteOptions::default()
        };
        remote_reboot_with_sudo_password(&runner, &opts, "secret").unwrap();
        let calls = runner.calls.lock().unwrap();
        assert!(calls[0].1.iter().any(|a| a.contains("sudo -S reboot")));
        drop(calls);
    }

    #[test]
    fn remote_ensure_ssh_key_and_reboot_wrappers() {
        crate::remote::ssh::tests::with_fake_default_pubkey(|_| {
            let runner = ScriptedRunner::default();
            // Probe: already authorized → skip ssh-copy-id.
            runner.push("ssh", ScriptedRunner::ok(""));
            let opts = RemoteOptions {
                host: "box".into(),
                ..RemoteOptions::default()
            };
            remote_ensure_ssh_key(&runner, &opts).unwrap();

            let runner2 = ScriptedRunner::default();
            runner2.push("ssh", ScriptedRunner::ok(""));
            remote_reboot(&runner2, &opts).unwrap();
        });
    }

    #[test]
    fn remote_run_offers_reboot_on_success_non_tty() {
        let stubs = bin_dir_with_stubs();
        let runner = ScriptedRunner::default();
        runner.push("ssh", ScriptedRunner::ok("x86_64\n"));
        push_cli_probes_missing(&runner);
        runner.push("ssh", ScriptedRunner::ok(""));
        runner.push("scp", ScriptedRunner::ok(""));
        runner.push("ssh", ScriptedRunner::ok(""));
        push_cli_probe_current(&runner);
        runner.push("ssh", ScriptedRunner::ok("done\n"));
        remote_run_cli(
            &runner,
            &RemoteRunRequest {
                options: RemoteOptions {
                    host: "box".into(),
                    bin_dir: Some(stubs.path().to_path_buf()),
                    ..RemoteOptions::default()
                },
                cli_args: vec!["doctor".into()],
                flags: RemoteRunFlags {
                    use_sudo: false,
                    install_payload_on_success: false,
                    offer_reboot_on_success: true,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem: EcosystemInstallChoice::none(),
            },
        )
        .unwrap();
    }
}
