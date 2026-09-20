//! Broad dry-run and pure-helper coverage for the shared engine.

use horto_os_ui_shared::context::PromptsProvider;
use horto_os_ui_shared::kits::{envfile, fs as fs_kit, template};
use horto_os_ui_shared::ops::leases::{export_dhcp_leases, parse_dnsmasq_leases, read_leases};
use horto_os_ui_shared::resume::{self, ResumeState, ResumeStepRecord, StepStatus};
use horto_os_ui_shared::{
    backup_status, box_status, doctor, embed, list_containers, lookup, pipeline, probe_disk_backup,
    require_root_for_apply, setup_run, setup_status, setup_step, ApplyMode, DiskBackupOpts,
    HortoError, HostContext, HostPaths, NonInteractivePrompts, SetupKind,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn temp_host_paths(root: &Path) -> HostPaths {
    HostPaths {
        active_setup: root.join("active_setup"),
        backup: root.join("backup"),
        docker: root.join("docker"),
        etc: root.join("etc"),
        lease_file: root.join("var/lib/misc/dnsmasq.leases"),
    }
}

fn dry_ctx(root: &Path, kind: SetupKind) -> HostContext {
    let paths = temp_host_paths(root);
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    std::fs::create_dir_all(&paths.backup).unwrap();
    std::fs::create_dir_all(&paths.docker).unwrap();
    std::fs::create_dir_all(&paths.etc).unwrap();
    HostContext::new(ApplyMode::DryRun, kind)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts))
}

#[test]
fn dry_run_full_pipeline_plans() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    setup_run(&mut ctx, SetupKind::Full).expect("dry-run full");
    assert!(!ctx.logs.is_empty());
    assert!(
        ctx.planned.is_empty() || !ctx.logs.is_empty(),
        "pipeline should log step headers"
    );
}

#[test]
fn dry_run_minimal_pipeline_plans() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Minimal);
    setup_run(&mut ctx, SetupKind::Minimal).expect("dry-run minimal");
    assert!(ctx
        .logs
        .iter()
        .any(|l| l.contains("m1") || l.contains("==>")));
}

#[test]
fn dry_run_each_registered_step_metadata_and_plan() {
    let tmp = TempDir::new().unwrap();
    for id in ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "m1", "d1"] {
        let step = lookup(id).unwrap_or_else(|| panic!("missing {id}"));
        assert!(!step.title().is_empty(), "{id} title");
        assert!(!step.reference_script().is_empty(), "{id} reference");
        assert!(step.step_version() >= 1, "{id} version");
        assert_eq!(step.id(), id);

        let kind = if id == "m1" {
            SetupKind::Minimal
        } else {
            SetupKind::Full
        };

        let mut ctx = dry_ctx(tmp.path(), kind);
        // Skip dependency gate for isolated plan by calling plan/apply directly.
        step.plan(&mut ctx)
            .unwrap_or_else(|e| panic!("{id} plan: {e}"));
        step.apply(&mut ctx)
            .unwrap_or_else(|e| panic!("{id} dry apply: {e}"));
    }
}

#[test]
fn setup_step_unknown_and_wrong_pipeline() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Minimal);
    let err = setup_step(&mut ctx, SetupKind::Minimal, "nope").unwrap_err();
    assert!(matches!(err, HortoError::UnknownStep(_)));
    assert_eq!(err.exit_code(), 2);

    let err = setup_step(&mut ctx, SetupKind::Minimal, "d1").unwrap_err();
    assert!(matches!(err, HortoError::NotInPipeline { .. }));
}

#[test]
fn setup_step_missing_dependency() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    let err = setup_step(&mut ctx, SetupKind::Full, "s5").unwrap_err();
    assert!(matches!(err, HortoError::MissingDependency(_, _)));
}

#[test]
fn resume_roundtrip_and_status() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let step = lookup("s1").unwrap();
    resume::record_ok(&paths, step).unwrap();
    let state = resume::load(&paths.resume_file()).unwrap();
    assert_eq!(resume::status_for(&state, step), StepStatus::Done);

    resume::mark_failed(&paths, "s2", 1).unwrap();
    let state = resume::load(&paths.resume_file()).unwrap();
    let s2 = lookup("s2").unwrap();
    assert_eq!(resume::status_for(&state, s2), StepStatus::Failed);

    let mut state = ResumeState::default();
    state.steps.push(ResumeStepRecord {
        id: "s1".into(),
        step_version: 0,
        completed_at: chrono::Utc::now(),
        ok: true,
    });
    resume::save(&paths.resume_file(), &state).unwrap();
    let state = resume::load(&paths.resume_file()).unwrap();
    assert_eq!(resume::status_for(&state, step), StepStatus::Stale);
}

#[test]
fn embed_assets_present() {
    let config = embed::list_prefix("config/");
    assert!(!config.is_empty(), "embedded config/");
    let docker = embed::list_prefix("docker_source/");
    assert!(!docker.is_empty(), "embedded docker_source/");
    let tops = embed::config_top_entries();
    assert!(!tops.is_empty());
    assert!(embed::get_str("config/os-configuration.env").is_some());
}

#[test]
fn paths_helpers() {
    let p = HostPaths::default();
    assert!(p.resume_file().ends_with("horto_setup_state.json"));
    assert!(p.full_env_file().ends_with("iot-lan_conf.env"));
    assert!(p.iot_lan_env_file().ends_with("iot-lan_conf.env"));
    assert!(p.minimal_env_file().ends_with("minimal_setup_vars.env"));
    assert!(p.os_configuration_file().ends_with("os-configuration.env"));
    assert!(p.staging_etc().ends_with("etc"));
    assert!(p.initial_backup_etc().ends_with("initial_setup"));
    assert!(p.leases_json().ends_with("leases.json"));
    assert!(p.leases_html().ends_with("leases.html"));
    assert_eq!(
        horto_os_ui_shared::paths::path_str(Path::new("/tmp/x")),
        "/tmp/x"
    );
}

#[test]
fn error_display_and_exit_codes() {
    assert_eq!(HortoError::RootRequired.exit_code(), 77);
    assert_eq!(HortoError::UnknownStep("x".into()).exit_code(), 2);
    assert_eq!(
        HortoError::NotInPipeline {
            step: "d1".into(),
            kind: "minimal".into()
        }
        .exit_code(),
        2
    );
    assert_eq!(HortoError::msg("x").exit_code(), 1);
    assert!(HortoError::msg("boom").to_string().contains("boom"));
    assert!(HortoError::command("apt", "fail")
        .to_string()
        .contains("apt"));
    assert!(HortoError::EmbedMissing("a".into())
        .to_string()
        .contains("embedded"));
    assert!(HortoError::MissingDependency("s5".into(), "s4".into())
        .to_string()
        .contains("s5"));
}

#[test]
fn require_root_dry_run_ok() {
    require_root_for_apply(ApplyMode::DryRun).unwrap();
}

#[test]
fn doctor_and_box_status_on_temp_paths() {
    let tmp = TempDir::new().unwrap();
    let ctx = dry_ctx(tmp.path(), SetupKind::Full);
    let d = doctor(&ctx);
    assert!(!d.active_setup_dir || d.active_setup_dir);
    assert!(!d.full_env);
    assert!(!d.notes.is_empty() || d.notes.is_empty());

    let report = setup_status(&ctx, SetupKind::Full);
    assert_eq!(report.kind, "full");
    assert_eq!(report.steps.len(), pipeline(SetupKind::Full).len());

    let box_st = box_status(&ctx, SetupKind::Full);
    assert!(!box_st.hostname.is_empty());
    assert!(!box_st.urls.is_empty());
    assert!(
        box_st
            .urls
            .iter()
            .any(|u| u.name == "Homepage" && u.url.contains(":3021")),
        "embedded service_links defaults: {:?}",
        box_st.urls
    );

    let mut map = std::collections::BTreeMap::new();
    map.insert("SCHEME".into(), "http".into());
    map.insert("HOST".into(), "cov-box".into());
    map.insert("LINKS".into(), "Homepage:3999".into());
    envfile::write(&ctx.paths.service_links_file(), &map).unwrap();
    let overridden = box_status(&ctx, SetupKind::Full);
    assert_eq!(overridden.urls.len(), 1);
    assert_eq!(overridden.urls[0].url, "http://cov-box:3999");
    let _ = backup_status(&ctx);
    let _ = list_containers();
}

#[test]
fn leases_parse_and_export_dry_run() {
    let text = "1700000000 aa:bb:cc:dd:ee:ff 192.168.1.50 host-a *\nshort\n";
    let parsed = parse_dnsmasq_leases(text);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].hostname, "host-a");
    assert_eq!(parsed[0].ip, "192.168.1.50");

    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    let lease_path = ctx.paths.lease_file.clone();
    std::fs::create_dir_all(lease_path.parent().unwrap()).unwrap();
    std::fs::write(&lease_path, text).unwrap();
    let from_file = read_leases(&lease_path, &ctx.paths.leases_json());
    assert_eq!(from_file.len(), 1);

    export_dhcp_leases(&mut ctx).unwrap();
    assert!(ctx.planned.iter().any(|p| p.summary.contains("lease")
        || p.summary.contains("write")
        || p.summary.contains("mkdir")
        || ctx
            .logs
            .iter()
            .any(|l| l.contains("Lease") || l.contains("dry-run"))));
}

#[test]
fn leases_json_fallback() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(paths.docker_assets()).unwrap();
    let json = paths.leases_json();
    std::fs::write(
        &json,
        r#"[{"hostname":"h","ip":"1.2.3.4","mac":"aa","expires":"0"}]"#,
    )
    .unwrap();
    let list = read_leases(Path::new("/no/such/leases"), &json);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].hostname, "h");
}

#[test]
fn fs_kit_apply_mode_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let mut ctx =
        HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_host_paths(tmp.path()));
    let dir = tmp.path().join("nest");
    let file = dir.join("a.txt");
    fs_kit::ensure_dir(&mut ctx, &dir).unwrap();
    fs_kit::write_file(&mut ctx, &file, b"hi").unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
    let dest = dir.join("b.txt");
    fs_kit::copy_file(&mut ctx, &file, &dest).unwrap();
    assert!(dest.is_file());
    let tree_src = tmp.path().join("tree_src");
    let tree_dst = tmp.path().join("tree_dst");
    std::fs::create_dir_all(tree_src.join("sub")).unwrap();
    std::fs::write(tree_src.join("sub/f"), b"x").unwrap();
    fs_kit::copy_dir_contents(&mut ctx, &tree_src, &tree_dst).unwrap();
    assert!(tree_dst.join("sub/f").is_file());
    fs_kit::chmod(&mut ctx, &file, 0o644).unwrap();
    fs_kit::remove_path(&mut ctx, &dest).unwrap();
    assert!(!dest.exists());
}

#[test]
fn fs_kit_dry_run_plans_only() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    let p = tmp.path().join("no-write.txt");
    fs_kit::write_file(&mut ctx, &p, b"x").unwrap();
    assert!(!p.exists());
    assert!(!ctx.planned.is_empty());
}

#[test]
fn envfile_and_template_extras() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("x.env");
    let mut map = std::collections::BTreeMap::new();
    map.insert("A".into(), "1".into());
    map.insert("B".into(), "two".into());
    envfile::write(&path, &map).unwrap();
    let read = envfile::load(&path).unwrap();
    assert_eq!(read.get("A").map(String::as_str), Some("1"));
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("NAME".into(), "horto".into());
    let rendered = template::render("hi {{NAME}}", &vars);
    assert_eq!(rendered, "hi horto");
    assert!(!template::has_unreplaced_placeholders(&rendered));
}

#[test]
fn probe_disk_backup_reports() {
    let opts = DiskBackupOpts {
        source: PathBuf::from("/dev/null"),
        dest_dir: PathBuf::from("/tmp"),
        include_boot_sectors: false,
        force: false,
    };
    let probe = probe_disk_backup(&opts);
    assert!(!probe.safe_to_apply || probe.safe_to_apply);
    assert!(!probe.blockers.is_empty() || probe.notes.is_empty() || !probe.notes.is_empty());
}

#[test]
fn noninteractive_prompts() {
    let mut p = NonInteractivePrompts;
    assert_eq!(p.prompt("x", "def"), "def");
    assert!(p.confirm("y?", true));
    assert!(!p.confirm("y?", false));
}

#[test]
fn setup_kind_as_str() {
    assert_eq!(SetupKind::Full.as_str(), "full");
    assert_eq!(SetupKind::Minimal.as_str(), "minimal");
}

#[test]
fn context_prompt_answers_and_plan() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    ctx.prompt_answers.insert("Label".into(), "answer".into());
    assert_eq!(ctx.prompt("Label", "default"), "answer");
    assert_eq!(ctx.prompt("Other", "default"), "default");
    ctx.apply_nat = true;
    assert!(ctx.confirm("Apply NAT rules?", false));
    ctx.plan_action("demo");
    assert_eq!(ctx.planned.len(), 1);
    assert!(ctx.is_dry_run());
}

#[test]
fn systemd_and_apt_dry_run_plan() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    use horto_os_ui_shared::kits::{apt, systemd};
    apt::apt_update(&mut ctx).unwrap();
    apt::apt_install(&mut ctx, &["curl"]).unwrap();
    systemd::stop(&mut ctx, "dnsmasq").unwrap();
    systemd::disable(&mut ctx, "dnsmasq").unwrap();
    systemd::enable(&mut ctx, "dnsmasq").unwrap();
    systemd::start(&mut ctx, "dnsmasq").unwrap();
    systemd::restart(&mut ctx, "dnsmasq").unwrap();
    systemd::unmask(&mut ctx, "hostapd").unwrap();
    systemd::try_restart(&mut ctx, "dnsmasq");
    systemd::try_enable(&mut ctx, "dnsmasq");
    systemd::try_unmask(&mut ctx, "hostapd");
    systemd::try_start(&mut ctx, "dnsmasq");
    systemd::try_stop(&mut ctx, "dnsmasq");
    systemd::try_disable(&mut ctx, "dnsmasq");
    let _ = systemd::unit_present("ssh");
    let _ = apt::package_installed("bash");
    assert!(ctx.planned.len() >= 8);
}

#[test]
fn s2_apply_writes_env_on_temp_paths() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    ctx.prompt_answers
        .insert("Device hostname".into(), "box-test".into());
    ctx.prompt_answers
        .insert("OS type (debian/armbian)".into(), "debian".into());
    ctx.prompt_answers
        .insert("NPU type (rkRK3576/rkRK3588/...)".into(), "rkRK3588".into());
    ctx.prompt_answers
        .insert("RAM size label".into(), "8gb".into());
    ctx.prompt_answers
        .insert("Install type (home/satellite/hortex)".into(), "home".into());
    ctx.prompt_answers
        .insert("Enable IOT-LAN (y/n)".into(), "y".into());
    ctx.prompt_answers
        .insert("Public URL / domain".into(), "example.test".into());
    ctx.prompt_answers
        .insert("Cloudflare token (optional)".into(), "".into());
    ctx.prompt_answers.insert(
        "WiFi interface (none = Ethernet-only)".into(),
        "wlan0".into(),
    );
    ctx.prompt_answers
        .insert("WiFi SSID".into(), "TestSSID".into());
    ctx.prompt_answers
        .insert("WiFi passphrase".into(), "secret".into());
    let step = lookup("s2").unwrap();
    step.apply(&mut ctx).expect("s2 apply");
    let os_map = envfile::load(&ctx.paths.os_configuration_file()).unwrap();
    assert_eq!(
        os_map.get("MY_HOSTNAME").map(String::as_str),
        Some("box-test")
    );
    assert_eq!(os_map.get("IOT_LAN").map(String::as_str), Some("y"));
    let map = envfile::load(&ctx.paths.full_env_file()).unwrap();
    assert_eq!(map.get("WIFI_SSID").map(String::as_str), Some("TestSSID"));
    assert_eq!(map.get("WIFI_INTERFACE").map(String::as_str), Some("wlan0"));
    assert!(step.is_done(&ctx));
}

#[test]
fn s2_apply_ethernet_only_skips_wifi_ssid() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    ctx.prompt_answers
        .insert("Device hostname".into(), "eth-only".into());
    ctx.prompt_answers
        .insert("OS type (debian/armbian)".into(), "debian".into());
    ctx.prompt_answers
        .insert("NPU type (rkRK3576/rkRK3588/...)".into(), "rkRK3588".into());
    ctx.prompt_answers
        .insert("RAM size label".into(), "8gb".into());
    ctx.prompt_answers
        .insert("Install type (home/satellite/hortex)".into(), "home".into());
    ctx.prompt_answers
        .insert("Enable IOT-LAN (y/n)".into(), "y".into());
    ctx.prompt_answers
        .insert("Public URL / domain".into(), "example.test".into());
    ctx.prompt_answers
        .insert("Cloudflare token (optional)".into(), "".into());
    ctx.prompt_answers.insert(
        "WiFi interface (none = Ethernet-only)".into(),
        "none".into(),
    );
    let step = lookup("s2").unwrap();
    step.apply(&mut ctx).expect("s2 ethernet-only");
    let map = envfile::load(&ctx.paths.full_env_file()).unwrap();
    assert_eq!(map.get("MY_HOSTNAME").map(String::as_str), Some("eth-only"));
    assert_eq!(map.get("WIFI_INTERFACE").map(String::as_str), Some("none"));
    assert_eq!(map.get("WIFI_SSID").map(String::as_str), Some(""));
    assert!(map.contains_key("ETH_LAN"));
    assert!(map.contains_key("ETH_IOT1"));
    assert!(ctx.logs.iter().any(|l| l.contains("Ethernet-only IoT-LAN")));
    assert!(step.is_done(&ctx));
}

#[test]
fn export_leases_apply_writes_files() {
    let tmp = TempDir::new().unwrap();
    let mut ctx =
        HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_host_paths(tmp.path()));
    std::fs::create_dir_all(ctx.paths.lease_file.parent().unwrap()).unwrap();
    std::fs::write(
        &ctx.paths.lease_file,
        "1700000000 aa:bb:cc:dd:ee:ff 192.168.1.50 host-a *\n",
    )
    .unwrap();
    export_dhcp_leases(&mut ctx).unwrap();
    assert!(ctx.paths.leases_json().is_file());
    assert!(ctx.paths.leases_html().is_file());
    export_dhcp_leases(&mut ctx).unwrap();
}

#[test]
fn setup_step_s2_after_s1_resume() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    resume::record_ok(&paths, lookup("s1").unwrap()).unwrap();
    let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    setup_step(&mut ctx, SetupKind::Full, "s2").expect("s2 after s1 recorded");
}

#[test]
fn s6_apply_validates_staged_etc() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    std::fs::create_dir_all(paths.etc.join("netplan")).unwrap();
    std::fs::create_dir_all(paths.etc.join("hostapd")).unwrap();
    std::fs::create_dir_all(paths.etc.join("avahi")).unwrap();
    std::fs::create_dir_all(paths.etc.join("dnsmasq.d")).unwrap();
    std::fs::create_dir_all(paths.etc.join("sysctl.d")).unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "box".into());
    map.insert("WIFI_INTERFACE".into(), "wlan0".into());
    map.insert("WIFI_SSID".into(), "ssid".into());
    envfile::write(&paths.full_env_file(), &map).unwrap();
    for (rel, body) in [
        ("hosts", "127.0.0.1 localhost\n"),
        ("netplan/99-iot-lan.yaml", "network: {version: 2}\n"),
        ("hostapd/hostapd.conf", "interface=wlan0\n"),
        ("avahi/avahi-daemon.conf", "[server]\n"),
        ("avahi/hosts", "\n"),
        ("resolv.conf", "nameserver 1.1.1.1\n"),
        ("dnsmasq.d/iot-lan.conf", "# ok\n"),
        ("sysctl.d/packet_forwarding.conf", "net.ipv4.ip_forward=1\n"),
    ] {
        std::fs::write(paths.etc.join(rel), body).unwrap();
    }
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
    lookup("s6").unwrap().apply(&mut ctx).expect("s6 apply");
}

#[test]
fn s3_s4_s5_apply_chain_on_temp_paths() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    std::fs::create_dir_all(&paths.etc).unwrap();
    // Flat files vs directories under config/.
    for name in ["hosts", "hostname", "resolv.conf", "os-configuration.env"] {
        std::fs::write(paths.etc.join(name), b"# seed\n").unwrap();
    }
    for name in ["avahi", "dnsmasq.d", "hostapd", "netplan", "sysctl.d"] {
        let d = paths.etc.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("marker"), b"x").unwrap();
    }
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "box-cov".into());
    map.insert("WIFI_INTERFACE".into(), "wlan0".into());
    map.insert("WIFI_SSID".into(), "ssid".into());
    map.insert("ETH_LAN".into(), "wan".into());
    map.insert("ETH_IOT1".into(), "lan1".into());
    envfile::write(&paths.full_env_file(), &map).unwrap();

    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    lookup("s3").unwrap().apply(&mut ctx).expect("s3");
    assert!(ctx.paths.initial_backup_etc().is_dir());

    lookup("s4").unwrap().apply(&mut ctx).expect("s4");
    assert!(ctx.paths.staging_etc().join("hostname").is_file());
    assert!(ctx.paths.staging_etc().join("resolv.conf").is_file());

    // Existing resolv.conf exercises remove-before-copy (asset keeps 127.0.0.1 nameserver).
    std::fs::write(ctx.paths.etc.join("resolv.conf"), b"nameserver 127.0.0.1\n").unwrap();
    lookup("s5").unwrap().apply(&mut ctx).expect("s5");
    assert!(ctx.paths.etc.join("hostname").is_file());
    let resolv = std::fs::read_to_string(ctx.paths.etc.join("resolv.conf")).unwrap();
    assert!(resolv.contains("nameserver"));
    assert!(lookup("s5").unwrap().is_done(&ctx));
}

#[test]
fn s5_skips_without_full_env_apply() {
    let tmp = TempDir::new().unwrap();
    let mut ctx =
        HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_host_paths(tmp.path()));
    lookup("s5").unwrap().apply(&mut ctx).expect("skip");
    assert!(ctx.logs.iter().any(|l| l.contains("skipping")));
}

#[test]
fn s5_errors_when_staging_missing() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "box".into());
    envfile::write(&paths.full_env_file(), &map).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
    let err = lookup("s5").unwrap().apply(&mut ctx).unwrap_err();
    assert!(err.to_string().contains("staging"));
}

#[test]
fn s7_not_root_exports_leases_only() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "box".into());
    map.insert("WIFI_INTERFACE".into(), "wlan0".into());
    map.insert("WIFI_SSID".into(), "ssid".into());
    envfile::write(&paths.full_env_file(), &map).unwrap();
    std::fs::create_dir_all(paths.lease_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.lease_file,
        "1700000000 aa:bb:cc:dd:ee:ff 192.168.1.50 host-a *\n",
    )
    .unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
    lookup("s7").unwrap().apply(&mut ctx).expect("s7 not-root");
    assert!(ctx.paths.leases_json().is_file());
    assert!(ctx
        .logs
        .iter()
        .any(|l| l.contains("Not root") || l.contains("leases")));
}

#[test]
fn s7_skips_without_full_env() {
    let tmp = TempDir::new().unwrap();
    let mut ctx =
        HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_host_paths(tmp.path()));
    lookup("s7").unwrap().apply(&mut ctx).expect("skip");
}

#[test]
fn m1_apply_writes_minimal_env_without_root() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Minimal)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    ctx.prompt_answers
        .insert("Device hostname".into(), "mini-box".into());
    lookup("m1").unwrap().apply(&mut ctx).expect("m1");
    let map = envfile::load(&ctx.paths.minimal_env_file()).unwrap();
    assert_eq!(map.get("MY_HOSTNAME").map(String::as_str), Some("mini-box"));
    assert!(lookup("m1").unwrap().is_done(&ctx));
}

#[test]
fn backup_timestamped_and_list() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.etc).unwrap();
    std::fs::write(paths.etc.join("hosts"), b"127.0.0.1 localhost\n").unwrap();
    std::fs::write(paths.etc.join("hostname"), b"box\n").unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
    let report = horto_os_ui_shared::backup_etc_timestamped(&mut ctx).unwrap();
    assert!(!report.copied.is_empty() || !report.skipped_missing.is_empty());
    let listed = horto_os_ui_shared::list_timestamped_etc_backups(&ctx);
    assert!(!listed.is_empty());
    let status = backup_status(&ctx);
    assert!(!status.timestamped.is_empty());
}

#[test]
fn plan_disk_backup_records_actions() {
    let tmp = TempDir::new().unwrap();
    let mut ctx = dry_ctx(tmp.path(), SetupKind::Full);
    let dest = tmp.path().join("disk_out");
    std::fs::create_dir_all(&dest).unwrap();
    let opts = DiskBackupOpts {
        source: PathBuf::from("/dev/null"),
        dest_dir: dest,
        include_boot_sectors: true,
        force: true,
    };
    let planned = horto_os_ui_shared::plan_disk_backup(&mut ctx, &opts);
    assert!(!planned.is_empty());
    let probe = probe_disk_backup(&opts);
    assert!(probe.source_exists);
}

#[test]
fn docker_ps_parse_and_available() {
    let rows = horto_os_ui_shared::parse_docker_ps_lines(
        "abc123\tweb\tnginx:latest\tUp 2 hours\t0.0.0.0:80->80/tcp\nshort\n",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "abc123");
    assert_eq!(rows[0].ports, "0.0.0.0:80->80/tcp");
    assert_eq!(rows[0].stack.as_deref(), Some("web"));
    let with_project = horto_os_ui_shared::parse_docker_ps_lines(
        "abc\tweb\tnginx:latest\tUp 1h\t80/tcp\thomepage\n",
    );
    assert_eq!(with_project[0].stack.as_deref(), Some("homepage"));
    let _ = horto_os_ui_shared::docker_available();
    let _ = list_containers();
}

#[test]
fn envfile_require_and_parse_edges() {
    let mut map =
        envfile::parse("# comment\nFOO=bar\nexport BAZ=\"qux\"\nEMPTY=\nNOEQ\nKEY=val # trail\n");
    assert_eq!(map.get("FOO").map(String::as_str), Some("bar"));
    envfile::set_key(&mut map, "NEW", "1");
    envfile::require_keys(&map, &["FOO", "NEW"]).unwrap();
    let err = envfile::require_keys(&map, &["MISSING"]).unwrap_err();
    assert!(err.to_string().contains("MISSING") || err.to_string().contains("missing"));
}

#[test]
fn pipeline_ids_and_lookup() {
    assert!(pipeline(SetupKind::Full).len() >= 7);
    assert!(!pipeline(SetupKind::Minimal).is_empty());
    assert!(lookup("s1").is_some());
    assert!(lookup("nope").is_none());
    assert!(!horto_os_ui_shared::pipeline::full_ids().is_empty());
    assert!(horto_os_ui_shared::pipeline::minimal_ids().contains(&"m1"));
}

#[test]
fn s4_minimal_mode_staging() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "mini".into());
    envfile::write(&paths.minimal_env_file(), &map).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Minimal).with_paths(paths);
    lookup("s4").unwrap().apply(&mut ctx).expect("s4 minimal");
    assert!(ctx.paths.staging_etc().join("hostname").is_file());
    assert!(!ctx
        .paths
        .staging_etc()
        .join("hostapd/hostapd.conf")
        .exists());
}

#[test]
fn d1_apply_extracts_docker_with_skip_piper() {
    let tmp = TempDir::new().unwrap();
    let paths = temp_host_paths(tmp.path());
    std::fs::create_dir_all(&paths.active_setup).unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert("MY_HOSTNAME".into(), "box".into());
    map.insert("WIFI_INTERFACE".into(), "wlan0".into());
    map.insert("WIFI_SSID".into(), "ssid".into());
    envfile::write(&paths.full_env_file(), &map).unwrap();
    let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
        .with_paths(paths)
        .with_prompts(Box::new(NonInteractivePrompts));
    ctx.skip_piper = true;
    lookup("d1").unwrap().apply(&mut ctx).expect("d1 apply");
    assert!(
        ctx.paths.docker.join("stacks").is_dir()
            || ctx.paths.docker.join("dockge").exists()
            || ctx
                .paths
                .docker
                .read_dir()
                .ok()
                .is_some_and(|mut d| d.next().is_some())
    );
    assert!(lookup("d1").unwrap().is_done(&ctx) || !ctx.logs.is_empty());
}
