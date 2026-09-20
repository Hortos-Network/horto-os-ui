use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use horto_os_ui_shared::{
    backup_disk, backup_etc_initial, backup_etc_timestamped, backup_shrink, backup_status,
    docker_rebuild, doctor, export_dhcp_leases, footer_line, init_tracing, list_containers,
    list_timestamped_etc_backups, probe_disk_backup, read_leases, remote_run_cli,
    require_root_for_apply, setup_run, setup_status, setup_step, ApplyMode, DiskBackupOpts,
    HostContext, RemoteOptions, RemoteRunRequest, SetupKind, ShrinkBackupOpts, StdioPrompts,
    SystemProcessRunner, LONG_VERSION,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui",
    about = "Horto OS UI CLI",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    /// Plan actions without writing privileged paths
    #[arg(long, global = true)]
    dry_run: bool,

    /// Skip piper model download during docker init
    #[arg(long, global = true)]
    skip_piper: bool,

    /// OpenSSH Host alias or user@host; run setup/doctor on that box via SSH
    #[arg(long, global = true)]
    remote: Option<String>,

    /// Opt-in: install this PC's public key on the box (`ssh-copy-id`). Off by default.
    #[arg(long, global = true, default_value_t = false)]
    install_ssh_key: bool,

    /// Local directory with horto-os-ui / horto-os-ui-tui / horto-os-ui-status-api (skips GitHub)
    #[arg(long, global = true, env = "HORTO_BIN_DIR")]
    bin_dir: Option<PathBuf>,

    /// GitHub Release tag for box tar.gz download (`v0.1.0` or tip `dev-preview`)
    #[arg(long, global = true, env = "HORTO_RELEASE_TAG")]
    release_tag: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Setup pipeline status / run
    Setup {
        #[command(subcommand)]
        cmd: SetupCmd,
    },
    /// Host readiness checks
    Doctor,
    /// Docker helpers
    Docker {
        #[command(subcommand)]
        cmd: DockerCmd,
    },
    /// Network / DHCP lease helpers
    Net {
        #[command(subcommand)]
        cmd: NetCmd,
    },
    /// Config and full-disk backups
    Backup {
        #[command(subcommand)]
        cmd: BackupCmd,
    },
}

#[derive(Subcommand, Debug)]
enum SetupCmd {
    Status {
        #[arg(long, group = "kind")]
        full: bool,
        #[arg(long, group = "kind")]
        minimal: bool,
    },
    Run {
        #[arg(long, group = "kind")]
        full: bool,
        #[arg(long, group = "kind")]
        minimal: bool,
    },
    Step {
        id: String,
        #[arg(long, group = "kind")]
        full: bool,
        #[arg(long, group = "kind")]
        minimal: bool,
    },
}

#[derive(Subcommand, Debug)]
enum DockerCmd {
    Status,
    /// Run the d1 docker init step
    Init,
    /// Rebuild one compose project (cwd or --dir)
    Rebuild {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum NetCmd {
    Leases,
    #[command(name = "export-leases")]
    ExportLeases,
}

#[derive(Subcommand, Debug)]
enum BackupCmd {
    /// Timestamped managed /etc backup (or --initial for protected initial_setup)
    Etc {
        /// Write to /srv/backup/etc/initial_setup (same as setup step s3)
        #[arg(long)]
        initial: bool,
    },
    /// List timestamped /etc backups
    List,
    /// Show disk-backup readiness (always safe)
    #[command(name = "disk-status")]
    DiskStatus {
        #[arg(long, default_value = "/dev/mmcblk0p1")]
        source: PathBuf,
        #[arg(long, default_value = "/mnt/external/horto-os")]
        dest: PathBuf,
    },
    /// Guarded partclone eMMC partition backup (boot from SD first)
    Disk {
        #[arg(long, default_value = "/dev/mmcblk0p1")]
        source: PathBuf,
        #[arg(long, default_value = "/mnt/external/horto-os")]
        dest: PathBuf,
        /// Also dump first 4MiB of /dev/mmcblk0 (boot sectors)
        #[arg(long)]
        boot_sectors: bool,
        /// Allow when root is not clearly SD/USB (never when root is eMMC)
        #[arg(long)]
        force: bool,
    },
    /// Optional shrink-backup wrapper (tool must be on PATH)
    Shrink {
        #[arg(long)]
        dest: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// JSON status used by API / soft client
    Status,
}

fn kind_from_flags(full: bool, minimal: bool) -> SetupKind {
    if minimal {
        SetupKind::Minimal
    } else {
        let _ = full;
        SetupKind::Full
    }
}

fn mode(dry_run: bool) -> ApplyMode {
    if dry_run {
        ApplyMode::DryRun
    } else {
        ApplyMode::Apply
    }
}

fn make_ctx(cli: &Cli, kind: SetupKind) -> HostContext {
    let mut ctx = HostContext::new(mode(cli.dry_run), kind).with_prompts(Box::new(StdioPrompts));
    ctx.skip_piper = cli.skip_piper;
    ctx
}

fn remote_options(cli: &Cli) -> RemoteOptions {
    let mut opts = RemoteOptions {
        host: cli.remote.clone().unwrap_or_default(),
        install_ssh_key: cli.install_ssh_key,
        bin_dir: cli.bin_dir.clone(),
        ..RemoteOptions::default()
    };
    if let Some(tag) = cli
        .release_tag
        .as_ref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
    {
        opts.release_tag = tag.to_owned();
    }
    opts
}

fn remote_cli_args(cli: &Cli, rest: &[&str]) -> Vec<String> {
    let mut args = Vec::new();
    if cli.dry_run {
        args.push("--dry-run".into());
    }
    if cli.skip_piper {
        args.push("--skip-piper".into());
    }
    for a in rest {
        args.push((*a).to_owned());
    }
    args
}

fn run_remote(cli: &Cli, rest: &[&str], use_sudo: bool, install_payload: bool) -> Result<String> {
    let req = RemoteRunRequest {
        options: remote_options(cli),
        cli_args: remote_cli_args(cli, rest),
        use_sudo,
        install_payload_on_success: install_payload,
    };
    Ok(remote_run_cli(&SystemProcessRunner, &req)?)
}

fn main() -> Result<()> {
    init_tracing("info");
    let cli = Cli::parse();
    match &cli.command {
        Commands::Setup { cmd } => match cmd {
            SetupCmd::Status { full, minimal } => {
                if cli.remote.is_some() {
                    let kind = if *minimal { "--minimal" } else { "--full" };
                    let out = run_remote(&cli, &["setup", "status", kind], false, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let kind = kind_from_flags(*full, *minimal);
                    let ctx = make_ctx(&cli, kind);
                    let report = setup_status(&ctx, kind);
                    println!("Setup kind: {}", report.kind);
                    for s in &report.steps {
                        let flags = format!(
                            "{}{}",
                            if s.destructive { " [destructive]" } else { "" },
                            if s.needs_reboot_after {
                                " [reboot]"
                            } else {
                                ""
                            }
                        );
                        println!(
                            "  [{:>7}] {} - {} (v{}){flags}",
                            s.status, s.id, s.title, s.step_version
                        );
                    }
                }
            }
            SetupCmd::Run { full, minimal } => {
                if cli.remote.is_some() {
                    let kind = if *minimal { "--minimal" } else { "--full" };
                    let install_payload = !cli.dry_run;
                    let out =
                        run_remote(&cli, &["setup", "run", kind], !cli.dry_run, install_payload)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let kind = kind_from_flags(*full, *minimal);
                    let mut ctx = make_ctx(&cli, kind);
                    require_root_for_apply(ctx.mode).context("root check")?;
                    setup_run(&mut ctx, kind)?;
                }
            }
            SetupCmd::Step { id, full, minimal } => {
                if cli.remote.is_some() {
                    let kind = if *minimal { "--minimal" } else { "--full" };
                    let out = run_remote(&cli, &["setup", "step", id, kind], !cli.dry_run, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let kind = kind_from_flags(*full, *minimal);
                    let mut ctx = make_ctx(&cli, kind);
                    require_root_for_apply(ctx.mode).context("root check")?;
                    setup_step(&mut ctx, kind, id)?;
                }
            }
        },
        Commands::Doctor => {
            if cli.remote.is_some() {
                let out = run_remote(&cli, &["doctor"], false, false)?;
                if !out.is_empty() {
                    println!("{out}");
                }
            } else {
                let ctx = make_ctx(&cli, SetupKind::Full);
                let report = doctor(&ctx);
                println!("{}", serde_json::to_string_pretty(&report)?);
                eprintln!("{}", footer_line());
            }
        }
        Commands::Docker { cmd } => match cmd {
            DockerCmd::Status => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["docker", "status"], false, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let list = list_containers()?;
                    if list.is_empty() {
                        println!("No containers (docker missing or none running).");
                    } else {
                        for c in list {
                            println!(
                                "{}\t{}\t{}\t{}\t{}",
                                c.id, c.names, c.image, c.status, c.ports
                            );
                        }
                    }
                }
            }
            DockerCmd::Init => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["docker", "init"], !cli.dry_run, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let mut ctx = make_ctx(&cli, SetupKind::Full);
                    require_root_for_apply(ctx.mode).context("root check")?;
                    setup_step(&mut ctx, SetupKind::Full, "d1")?;
                }
            }
            DockerCmd::Rebuild { dir } => {
                if cli.remote.is_some() {
                    anyhow::bail!("docker rebuild over --remote is not supported yet; use embedded or SSH manually");
                }
                docker_rebuild(dir)?;
                println!("Rebuilt compose project in {}", dir.display());
            }
        },
        Commands::Net { cmd } => match cmd {
            NetCmd::Leases => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["net", "leases"], false, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let ctx = make_ctx(&cli, SetupKind::Full);
                    let leases = read_leases(&ctx.paths.lease_file, &ctx.paths.leases_json());
                    if leases.is_empty() {
                        println!("No leases found.");
                    } else {
                        for l in leases {
                            println!("{}\t{}\t{}\t{}", l.hostname, l.ip, l.mac, l.expires);
                        }
                    }
                }
            }
            NetCmd::ExportLeases => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["net", "export-leases"], !cli.dry_run, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let mut ctx = make_ctx(&cli, SetupKind::Full);
                    export_dhcp_leases(&mut ctx)?;
                }
            }
        },
        Commands::Backup { cmd } => match cmd {
            BackupCmd::Etc { initial } => {
                if cli.remote.is_some() {
                    let mut args = vec!["backup", "etc"];
                    if *initial {
                        args.push("--initial");
                    }
                    let out = run_remote(&cli, &args, !cli.dry_run, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let mut ctx = make_ctx(&cli, SetupKind::Full);
                    require_root_for_apply(ctx.mode).context("root check")?;
                    let report = if *initial {
                        backup_etc_initial(&mut ctx)?
                    } else {
                        backup_etc_timestamped(&mut ctx)?
                    };
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
            }
            BackupCmd::List => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["backup", "list"], false, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let ctx = make_ctx(&cli, SetupKind::Full);
                    let list = list_timestamped_etc_backups(&ctx);
                    if list.is_empty() {
                        println!("No timestamped /etc backups under /srv/backup/etc/");
                    } else {
                        for name in list {
                            println!("{name}");
                        }
                    }
                }
            }
            BackupCmd::DiskStatus { source, dest } => {
                if cli.remote.is_some() {
                    let source_s = source.display().to_string();
                    let dest_s = dest.display().to_string();
                    let out = run_remote(
                        &cli,
                        &[
                            "backup",
                            "disk-status",
                            "--source",
                            &source_s,
                            "--dest",
                            &dest_s,
                        ],
                        false,
                        false,
                    )?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let probe = probe_disk_backup(&DiskBackupOpts {
                        source: source.clone(),
                        dest_dir: dest.clone(),
                        include_boot_sectors: false,
                        force: false,
                    });
                    println!("{}", serde_json::to_string_pretty(&probe)?);
                }
            }
            BackupCmd::Disk {
                source,
                dest,
                boot_sectors,
                force,
            } => {
                if cli.remote.is_some() {
                    let mut args = vec![
                        "backup".into(),
                        "disk".into(),
                        "--source".into(),
                        source.display().to_string(),
                        "--dest".into(),
                        dest.display().to_string(),
                    ];
                    if *boot_sectors {
                        args.push("--boot-sectors".into());
                    }
                    if *force {
                        args.push("--force".into());
                    }
                    let rest: Vec<&str> = args.iter().map(String::as_str).collect();
                    let out = run_remote(&cli, &rest, !cli.dry_run, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let mut ctx = make_ctx(&cli, SetupKind::Full);
                    require_root_for_apply(ctx.mode).context("root check")?;
                    let opts = DiskBackupOpts {
                        source: source.clone(),
                        dest_dir: dest.clone(),
                        include_boot_sectors: *boot_sectors,
                        force: *force,
                    };
                    let path = backup_disk(&mut ctx, &opts)?;
                    println!("image: {}", path.display());
                }
            }
            BackupCmd::Shrink { dest, force } => {
                if cli.remote.is_some() {
                    anyhow::bail!("backup shrink over --remote is not supported yet");
                }
                let mut ctx = make_ctx(&cli, SetupKind::Full);
                require_root_for_apply(ctx.mode).context("root check")?;
                let path = backup_shrink(
                    &mut ctx,
                    &ShrinkBackupOpts {
                        dest_img: dest.clone(),
                        force: *force,
                    },
                )?;
                println!("image: {}", path.display());
            }
            BackupCmd::Status => {
                if cli.remote.is_some() {
                    let out = run_remote(&cli, &["backup", "status"], false, false)?;
                    if !out.is_empty() {
                        println!("{out}");
                    }
                } else {
                    let ctx = make_ctx(&cli, SetupKind::Full);
                    println!("{}", serde_json::to_string_pretty(&backup_status(&ctx))?);
                }
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_common_commands() {
        let cases = [
            vec!["horto-os-ui", "--dry-run", "setup", "status", "--full"],
            vec!["horto-os-ui", "setup", "status", "--minimal"],
            vec!["horto-os-ui", "setup", "run", "--full"],
            vec!["horto-os-ui", "setup", "step", "s1", "--full"],
            vec!["horto-os-ui", "doctor"],
            vec!["horto-os-ui", "docker", "status"],
            vec!["horto-os-ui", "docker", "init"],
            vec!["horto-os-ui", "docker", "rebuild", "--dir", "/tmp"],
            vec!["horto-os-ui", "net", "leases"],
            vec!["horto-os-ui", "net", "export-leases"],
            vec!["horto-os-ui", "backup", "status"],
            vec!["horto-os-ui", "backup", "list"],
            vec!["horto-os-ui", "backup", "etc"],
            vec!["horto-os-ui", "backup", "etc", "--initial"],
            vec!["horto-os-ui", "backup", "disk-status"],
            vec!["horto-os-ui", "backup", "disk", "--force"],
            vec!["horto-os-ui", "backup", "shrink", "--dest", "/tmp/x.img"],
            vec![
                "horto-os-ui",
                "--remote",
                "horto-box",
                "--dry-run",
                "setup",
                "run",
                "--full",
            ],
            vec![
                "horto-os-ui",
                "--remote",
                "user@192.168.1.10",
                "--install-ssh-key",
                "--bin-dir",
                "/tmp/bins",
                "doctor",
            ],
        ];
        for args in cases {
            Cli::try_parse_from(args).expect("parse");
        }
    }

    #[test]
    fn remote_defaults_key_off() {
        let cli = Cli::try_parse_from(["horto-os-ui", "--remote", "box", "doctor"]).unwrap();
        assert_eq!(cli.remote.as_deref(), Some("box"));
        assert!(!cli.install_ssh_key);
        assert!(cli.bin_dir.is_none());
    }

    #[test]
    fn kind_and_mode_helpers() {
        assert_eq!(kind_from_flags(true, false), SetupKind::Full);
        assert_eq!(kind_from_flags(false, true), SetupKind::Minimal);
        assert_eq!(mode(true), ApplyMode::DryRun);
        assert_eq!(mode(false), ApplyMode::Apply);
    }
}
