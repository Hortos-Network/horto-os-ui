//! Horto OS UI CLI (`horto-os-ui`): plan-by-default box setup and day-2 ops.
//!
//! Surfaces call [`horto_os_ui_shared`] for steps, kits, remote SSH, and status.
//! Privileged writes require `--apply` (or `APPLY=1` via Make). Interactive
//! confirm / reboot prompts stay on stderr; ops logs use `tracing`.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use horto_os_ui_shared::{
    backup_disk, backup_etc_initial, backup_etc_timestamped, backup_shrink, backup_status,
    docker_rebuild, doctor, export_dhcp_leases, footer_line, format_surfaces_report, init_tracing,
    install_ecosystem_after_embedded_apply, list_containers, list_timestamped_etc_backups,
    offer_save_api_token, probe_disk_backup, probe_surfaces, read_leases,
    remote_doctor_report_banner, remote_run_cli, require_root_for_apply, setup_run, setup_status,
    setup_step, ApplyMode, DiskBackupOpts, EcosystemInstallChoice, HostContext, RemoteOptions,
    RemoteOptionsInput, RemoteRunFlags, RemoteRunOutcome, RemoteRunRequest, SetupKind,
    ShrinkBackupOpts, StackOpts, StdioPrompts, SystemProcessRunner, DEFAULT_INSTALL_DIR,
    LONG_VERSION,
};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui",
    about = "Horto OS UI CLI",
    version,
    long_version = LONG_VERSION
)]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    /// Apply privileged changes (default: plan only, no writes)
    #[arg(long, global = true)]
    apply: bool,

    /// Skip piper model download during docker init
    #[arg(long, global = true)]
    skip_piper: bool,

    /// Optional Docker stacks for d3 (comma-separated: dockge,open-webui,evcc,whisper,deepseek,piper,openwakeword)
    #[arg(long, global = true, default_value = "", env = "HORTO_STACKS")]
    stacks: String,

    /// Install status-api (default on; accepted for remote argv passthrough)
    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    install_status_api: bool,

    /// Disable status-api install
    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    no_install_status_api: bool,

    /// Install MCP (default on; accepted for remote argv passthrough)
    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    install_mcp: bool,

    /// Disable MCP install
    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    no_install_mcp: bool,

    /// OpenSSH Host alias, /etc/hosts name, or user@host; run setup/doctor via SSH
    #[arg(long, global = true, env = "HORTO_REMOTE_HOST")]
    remote: Option<String>,

    /// Opt-in: install this PC's public key on the box (`ssh-copy-id`). Off by default.
    #[arg(long, global = true, default_value_t = false)]
    install_ssh_key: bool,

    /// Local directory with horto-os-ui / tui / status-api / mcp (skips GitHub)
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
    /// Probe SSH / CLI / API / MCP surfaces (same report as TUI / Desktop)
    Surfaces {
        /// Print `SurfaceProbeReport` as JSON
        #[arg(long)]
        json: bool,
    },
    /// List `/etc/hosts` LAN names and OpenSSH `Host` aliases
    #[command(name = "known-hosts")]
    KnownHosts {
        /// Print `KnownRemoteHost` rows as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SetupCmd {
    Status {
        #[arg(long, group = "kind")]
        full: bool,
        #[arg(long, group = "kind")]
        minimal: bool,
        /// Print setup status as JSON (for remote TUI / scripts).
        #[arg(long)]
        json: bool,
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
    /// Timestamped managed /etc backup (or --initial for protected `initial_setup`)
    Etc {
        /// Write to /`srv/backup/etc/initial_setup` (same as setup step s3)
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

const fn kind_from_flags(full: bool, minimal: bool) -> SetupKind {
    if minimal {
        SetupKind::Minimal
    } else {
        let _ = full;
        SetupKind::Full
    }
}

const fn mode(apply: bool) -> ApplyMode {
    if apply {
        ApplyMode::Apply
    } else {
        ApplyMode::DryRun
    }
}

fn make_ctx(cli: &Cli, kind: SetupKind) -> HostContext {
    let mut ctx = HostContext::new(mode(cli.apply), kind).with_prompts(Box::new(StdioPrompts));
    ctx.skip_piper = cli.skip_piper;
    ctx.stack_opts = StackOpts::parse_csv(&cli.stacks);
    ctx.ecosystem = ecosystem_from_cli(cli);
    ctx
}

const fn ecosystem_from_cli(cli: &Cli) -> EcosystemInstallChoice {
    // Default both on. `--no-install-*` wins; bare `--install-*` is for remote passthrough.
    let _ = (cli.install_status_api, cli.install_mcp);
    EcosystemInstallChoice {
        status_api: !cli.no_install_status_api,
        mcp: !cli.no_install_mcp,
    }
}

fn remote_options(cli: &Cli) -> RemoteOptions {
    RemoteOptions::from_input(RemoteOptionsInput {
        host: cli.remote.clone().unwrap_or_default(),
        install_ssh_key: cli.install_ssh_key,
        bin_dir: cli.bin_dir.clone(),
        release_tag: cli.release_tag.clone(),
        force_askpass: false,
    })
}

fn remote_cli_args(cli: &Cli, rest: &[&str]) -> Vec<String> {
    let mut args = Vec::new();
    if cli.apply {
        args.push("--apply".into());
    }
    if cli.skip_piper {
        args.push("--skip-piper".into());
    }
    if !cli.stacks.trim().is_empty() {
        args.push(format!("--stacks={}", cli.stacks.trim()));
    }
    if cli.no_install_status_api {
        args.push("--no-install-status-api".into());
    } else {
        args.push("--install-status-api".into());
    }
    if cli.no_install_mcp {
        args.push("--no-install-mcp".into());
    } else {
        args.push("--install-mcp".into());
    }
    for a in rest {
        args.push((*a).to_owned());
    }
    args
}

fn run_remote(
    cli: &Cli,
    rest: &[&str],
    use_sudo: bool,
    ecosystem: EcosystemInstallChoice,
    offer_reboot: bool,
    capture_output: bool,
) -> Result<RemoteRunOutcome> {
    let req = RemoteRunRequest {
        options: remote_options(cli),
        cli_args: remote_cli_args(cli, rest),
        flags: RemoteRunFlags {
            use_sudo,
            install_payload_on_success: ecosystem.any(),
            offer_reboot_on_success: offer_reboot,
            capture_output,
            ..Default::default()
        },
        ecosystem,
    };
    Ok(remote_run_cli(&SystemProcessRunner, &req)?)
}

/// Remote privilege flags that track CLI `--apply` with the same polarity.
///
/// After `--dry-run` was removed, some call sites briefly used `!cli.apply`, so
/// `--apply` disabled sudo and skipped payload. Never invert again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RemoteApplyPrivilege {
    use_sudo: bool,
    offer_reboot: bool,
}

impl RemoteApplyPrivilege {
    /// `--apply` → sudo (and reboot offer when requested by the caller).
    #[must_use]
    const fn from_apply(apply: bool) -> Self {
        Self {
            use_sudo: apply,
            offer_reboot: apply,
        }
    }
}

/// Ecosystem answers apply only on remote full apply; otherwise skip both services.
#[must_use]
const fn remote_ecosystem_for_setup_run(
    apply: bool,
    minimal: bool,
    answered: EcosystemInstallChoice,
) -> EcosystemInstallChoice {
    if apply && !minimal {
        answered
    } else {
        EcosystemInstallChoice::none()
    }
}

fn print_remote_log(out: &RemoteRunOutcome) {
    if !out.log.is_empty() {
        println!("{}", out.log);
    }
}

fn maybe_offer_save_token(out: &RemoteRunOutcome) -> Result<()> {
    if let Some(token) = out.api_token.as_deref() {
        let _saved = offer_save_api_token(token)?;
    }
    Ok(())
}

fn maybe_install_ecosystem_embedded(cli: &Cli, kind: SetupKind) -> Result<()> {
    if !cli.apply || kind != SetupKind::Full {
        return Ok(());
    }
    let choice = ecosystem_from_cli(cli);
    if !choice.any() {
        tracing::info!("Skipped status-api / MCP install");
        return Ok(());
    }
    let install_dir = std::path::Path::new(DEFAULT_INSTALL_DIR);
    match install_ecosystem_after_embedded_apply(&SystemProcessRunner, install_dir, choice) {
        Ok(token) => {
            tracing::info!(
                "Installed selected ecosystem services under {}",
                install_dir.display()
            );
            if let Some(hex) = token.as_deref() {
                let _ = offer_save_api_token(hex)?;
            }
            Ok(())
        }
        Err(e) => {
            tracing::error!("ecosystem install after full apply failed: {e}");
            Err(e.into())
        }
    }
}

fn main() -> Result<()> {
    init_tracing("info");
    let cli = Cli::parse();
    match &cli.command {
        Commands::Setup { cmd } => cmd_setup(&cli, cmd)?,
        Commands::Doctor => cmd_doctor(&cli)?,
        Commands::Docker { cmd } => cmd_docker(&cli, cmd)?,
        Commands::Net { cmd } => cmd_net(&cli, cmd)?,
        Commands::Backup { cmd } => cmd_backup(&cli, cmd)?,
        Commands::Surfaces { json } => cmd_surfaces(&cli, *json)?,
        Commands::KnownHosts { json } => cmd_known_hosts(*json)?,
    }
    Ok(())
}

fn cmd_setup(cli: &Cli, cmd: &SetupCmd) -> Result<()> {
    match cmd {
        SetupCmd::Status {
            full,
            minimal,
            json,
        } => {
            if cli.remote.is_some() {
                let kind = if *minimal { "--minimal" } else { "--full" };
                let mut args = vec!["setup", "status", kind];
                if *json {
                    args.push("--json");
                }
                let out = run_remote(
                    cli,
                    &args,
                    false,
                    EcosystemInstallChoice::none(),
                    false,
                    true,
                )?;
                print_remote_log(&out);
            } else {
                let kind = kind_from_flags(*full, *minimal);
                let ctx = make_ctx(cli, kind);
                let report = setup_status(&ctx, kind);
                if *json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
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
        }
        SetupCmd::Run { full, minimal } => {
            if cli.remote.is_some() {
                let kind = if *minimal { "--minimal" } else { "--full" };
                let answered = if cli.apply && !*minimal {
                    ecosystem_from_cli(cli)
                } else {
                    EcosystemInstallChoice::none()
                };
                let ecosystem = remote_ecosystem_for_setup_run(cli.apply, *minimal, answered);
                let priv_ = RemoteApplyPrivilege::from_apply(cli.apply);
                let out = run_remote(
                    cli,
                    &["setup", "run", kind],
                    priv_.use_sudo,
                    ecosystem,
                    priv_.offer_reboot,
                    false,
                )?;
                print_remote_log(&out);
                maybe_offer_save_token(&out)?;
            } else {
                let kind = kind_from_flags(*full, *minimal);
                let mut ctx = make_ctx(cli, kind);
                require_root_for_apply(ctx.mode).context("root check")?;
                setup_run(&mut ctx, kind)?;
                maybe_install_ecosystem_embedded(cli, kind)?;
            }
        }
        SetupCmd::Step { id, full, minimal } => {
            if cli.remote.is_some() {
                let kind = if *minimal { "--minimal" } else { "--full" };
                let out = run_remote(
                    cli,
                    &["setup", "step", id, kind],
                    RemoteApplyPrivilege::from_apply(cli.apply).use_sudo,
                    EcosystemInstallChoice::none(),
                    false,
                    false,
                )?;
                print_remote_log(&out);
            } else {
                let kind = kind_from_flags(*full, *minimal);
                let mut ctx = make_ctx(cli, kind);
                require_root_for_apply(ctx.mode).context("root check")?;
                setup_step(&mut ctx, kind, id)?;
            }
        }
    }
    Ok(())
}

fn cmd_doctor(cli: &Cli) -> Result<()> {
    if cli.remote.is_some() {
        let out = run_remote(
            cli,
            &["doctor"],
            false,
            EcosystemInstallChoice::none(),
            false,
            true,
        )?;
        remote_doctor_report_banner();
        print_remote_log(&out);
    } else {
        let ctx = make_ctx(cli, SetupKind::Full);
        let report = doctor(&ctx);
        println!("{}", serde_json::to_string_pretty(&report)?);
        eprintln!("{}", footer_line());
    }
    Ok(())
}

fn cmd_docker(cli: &Cli, cmd: &DockerCmd) -> Result<()> {
    match cmd {
        DockerCmd::Status => {
            if cli.remote.is_some() {
                let out = run_remote(
                    cli,
                    &["docker", "status"],
                    false,
                    EcosystemInstallChoice::none(),
                    false,
                    true,
                )?;
                print_remote_log(&out);
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
                let out = run_remote(
                    cli,
                    &["docker", "init"],
                    RemoteApplyPrivilege::from_apply(cli.apply).use_sudo,
                    EcosystemInstallChoice::none(),
                    false,
                    false,
                )?;
                print_remote_log(&out);
            } else {
                let mut ctx = make_ctx(cli, SetupKind::Full);
                require_root_for_apply(ctx.mode).context("root check")?;
                setup_step(&mut ctx, SetupKind::Full, "d1")?;
            }
        }
        DockerCmd::Rebuild { dir } => {
            if cli.remote.is_some() {
                anyhow::bail!(
                    "docker rebuild over --remote is not supported yet; use embedded or SSH manually"
                );
            }
            docker_rebuild(dir)?;
            println!("Rebuilt compose project in {}", dir.display());
        }
    }
    Ok(())
}

fn cmd_net(cli: &Cli, cmd: &NetCmd) -> Result<()> {
    match cmd {
        NetCmd::Leases => {
            if cli.remote.is_some() {
                let out = run_remote(
                    cli,
                    &["net", "leases"],
                    false,
                    EcosystemInstallChoice::none(),
                    false,
                    true,
                )?;
                print_remote_log(&out);
            } else {
                let ctx = make_ctx(cli, SetupKind::Full);
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
                let out = run_remote(
                    cli,
                    &["net", "export-leases"],
                    RemoteApplyPrivilege::from_apply(cli.apply).use_sudo,
                    EcosystemInstallChoice::none(),
                    false,
                    false,
                )?;
                print_remote_log(&out);
            } else {
                let mut ctx = make_ctx(cli, SetupKind::Full);
                export_dhcp_leases(&mut ctx)?;
            }
        }
    }
    Ok(())
}

fn cmd_backup(cli: &Cli, cmd: &BackupCmd) -> Result<()> {
    match cmd {
        BackupCmd::Etc { initial } => cmd_backup_etc(cli, *initial),
        BackupCmd::List => cmd_backup_list(cli),
        BackupCmd::DiskStatus { source, dest } => cmd_backup_disk_status(cli, source, dest),
        BackupCmd::Disk {
            source,
            dest,
            boot_sectors,
            force,
        } => cmd_backup_disk(cli, source, dest, *boot_sectors, *force),
        BackupCmd::Shrink { dest, force } => cmd_backup_shrink(cli, dest, *force),
        BackupCmd::Status => cmd_backup_status(cli),
    }
}

fn cmd_backup_etc(cli: &Cli, initial: bool) -> Result<()> {
    if cli.remote.is_some() {
        let mut args = vec!["backup", "etc"];
        if initial {
            args.push("--initial");
        }
        let out = run_remote(
            cli,
            &args,
            RemoteApplyPrivilege::from_apply(cli.apply).use_sudo,
            EcosystemInstallChoice::none(),
            false,
            false,
        )?;
        print_remote_log(&out);
    } else {
        let mut ctx = make_ctx(cli, SetupKind::Full);
        require_root_for_apply(ctx.mode).context("root check")?;
        let report = if initial {
            backup_etc_initial(&mut ctx)?
        } else {
            backup_etc_timestamped(&mut ctx)?
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}

fn cmd_backup_list(cli: &Cli) -> Result<()> {
    if cli.remote.is_some() {
        let out = run_remote(
            cli,
            &["backup", "list"],
            false,
            EcosystemInstallChoice::none(),
            false,
            true,
        )?;
        print_remote_log(&out);
    } else {
        let ctx = make_ctx(cli, SetupKind::Full);
        let list = list_timestamped_etc_backups(&ctx);
        if list.is_empty() {
            println!("No timestamped /etc backups under /srv/backup/etc/");
        } else {
            for name in list {
                println!("{name}");
            }
        }
    }
    Ok(())
}

fn cmd_backup_disk_status(cli: &Cli, source: &Path, dest: &Path) -> Result<()> {
    if cli.remote.is_some() {
        let source_s = source.display().to_string();
        let dest_s = dest.display().to_string();
        let out = run_remote(
            cli,
            &[
                "backup",
                "disk-status",
                "--source",
                &source_s,
                "--dest",
                &dest_s,
            ],
            false,
            EcosystemInstallChoice::none(),
            false,
            true,
        )?;
        print_remote_log(&out);
    } else {
        let probe = probe_disk_backup(&DiskBackupOpts {
            source: source.to_path_buf(),
            dest_dir: dest.to_path_buf(),
            include_boot_sectors: false,
            force: false,
        });
        println!("{}", serde_json::to_string_pretty(&probe)?);
    }
    Ok(())
}

fn cmd_backup_disk(
    cli: &Cli,
    source: &Path,
    dest: &Path,
    boot_sectors: bool,
    force: bool,
) -> Result<()> {
    if cli.remote.is_some() {
        let mut args = vec![
            "backup".into(),
            "disk".into(),
            "--source".into(),
            source.display().to_string(),
            "--dest".into(),
            dest.display().to_string(),
        ];
        if boot_sectors {
            args.push("--boot-sectors".into());
        }
        if force {
            args.push("--force".into());
        }
        let rest: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run_remote(
            cli,
            &rest,
            RemoteApplyPrivilege::from_apply(cli.apply).use_sudo,
            EcosystemInstallChoice::none(),
            false,
            false,
        )?;
        print_remote_log(&out);
    } else {
        let mut ctx = make_ctx(cli, SetupKind::Full);
        require_root_for_apply(ctx.mode).context("root check")?;
        let opts = DiskBackupOpts {
            source: source.to_path_buf(),
            dest_dir: dest.to_path_buf(),
            include_boot_sectors: boot_sectors,
            force,
        };
        let path = backup_disk(&mut ctx, &opts)?;
        println!("image: {}", path.display());
    }
    Ok(())
}

fn cmd_backup_shrink(cli: &Cli, dest: &Path, force: bool) -> Result<()> {
    if cli.remote.is_some() {
        anyhow::bail!("backup shrink over --remote is not supported yet");
    }
    let mut ctx = make_ctx(cli, SetupKind::Full);
    require_root_for_apply(ctx.mode).context("root check")?;
    let path = backup_shrink(
        &mut ctx,
        &ShrinkBackupOpts {
            dest_img: dest.to_path_buf(),
            force,
        },
    )?;
    println!("image: {}", path.display());
    Ok(())
}

fn cmd_backup_status(cli: &Cli) -> Result<()> {
    if cli.remote.is_some() {
        let out = run_remote(
            cli,
            &["backup", "status"],
            false,
            EcosystemInstallChoice::none(),
            false,
            true,
        )?;
        print_remote_log(&out);
    } else {
        let ctx = make_ctx(cli, SetupKind::Full);
        println!("{}", serde_json::to_string_pretty(&backup_status(&ctx))?);
    }
    Ok(())
}

fn cmd_surfaces(cli: &Cli, json: bool) -> Result<()> {
    let embedded = cli.remote.is_none();
    let opts = if embedded {
        RemoteOptions::default()
    } else {
        remote_options(cli)
    };
    let report = probe_surfaces(&SystemProcessRunner, &opts, embedded)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", format_surfaces_report(&report));
        eprintln!("{}", footer_line());
    }
    Ok(())
}

fn cmd_known_hosts(json: bool) -> Result<()> {
    let hosts = horto_os_ui_shared::list_known_remote_hosts()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&hosts)?);
    } else {
        for host in hosts {
            println!("{}", host.name);
        }
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
            vec!["horto-os-ui", "setup", "status", "--full"],
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
            vec!["horto-os-ui", "surfaces"],
            vec!["horto-os-ui", "surfaces", "--json"],
            vec!["horto-os-ui", "known-hosts"],
            vec!["horto-os-ui", "known-hosts", "--json"],
            vec!["horto-os-ui", "--remote", "horto-box", "surfaces"],
            vec![
                "horto-os-ui",
                "--remote",
                "horto-box",
                "--apply",
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
    fn remote_from_horto_remote_host_env() {
        // clap reads HORTO_REMOTE_HOST when --remote is omitted.
        // Isolate from the process environment so CI hosts do not leak in.
        let prev = std::env::var_os("HORTO_REMOTE_HOST");
        std::env::set_var("HORTO_REMOTE_HOST", "env-box");
        let cli = Cli::try_parse_from(["horto-os-ui", "doctor"]).unwrap();
        assert_eq!(cli.remote.as_deref(), Some("env-box"));
        // Explicit --remote wins over env.
        let cli = Cli::try_parse_from(["horto-os-ui", "--remote", "flag-box", "doctor"]).unwrap();
        assert_eq!(cli.remote.as_deref(), Some("flag-box"));
        match prev {
            Some(v) => std::env::set_var("HORTO_REMOTE_HOST", v),
            None => std::env::remove_var("HORTO_REMOTE_HOST"),
        }
    }

    #[test]
    fn kind_and_mode_helpers() {
        assert_eq!(kind_from_flags(true, false), SetupKind::Full);
        assert_eq!(kind_from_flags(false, true), SetupKind::Minimal);
        assert_eq!(mode(false), ApplyMode::DryRun);
        assert_eq!(mode(true), ApplyMode::Apply);
    }

    #[test]
    fn remote_apply_privilege_is_not_inverted() {
        // Regression: leftover `!cli.apply` after dry-run → --apply rename.
        assert_eq!(
            RemoteApplyPrivilege::from_apply(true),
            RemoteApplyPrivilege {
                use_sudo: true,
                offer_reboot: true,
            }
        );
        assert_eq!(
            RemoteApplyPrivilege::from_apply(false),
            RemoteApplyPrivilege {
                use_sudo: false,
                offer_reboot: false,
            }
        );
        assert!(RemoteApplyPrivilege::from_apply(true).use_sudo);
        assert!(!RemoteApplyPrivilege::from_apply(false).use_sudo);
    }

    #[test]
    fn remote_ecosystem_only_on_full_apply() {
        let both = EcosystemInstallChoice {
            status_api: true,
            mcp: true,
        };
        assert_eq!(remote_ecosystem_for_setup_run(true, false, both), both);
        assert_eq!(
            remote_ecosystem_for_setup_run(false, false, both),
            EcosystemInstallChoice::none()
        );
        assert_eq!(
            remote_ecosystem_for_setup_run(true, true, both),
            EcosystemInstallChoice::none()
        );
    }

    #[test]
    fn parses_apply_flag_on_remote_setup_run() {
        let cli = Cli::try_parse_from([
            "horto-os-ui",
            "--remote",
            "box",
            "--apply",
            "setup",
            "run",
            "--full",
        ])
        .unwrap();
        assert!(cli.apply);
        let priv_ = RemoteApplyPrivilege::from_apply(cli.apply);
        assert!(priv_.use_sudo);
        assert!(priv_.offer_reboot);
    }
}
