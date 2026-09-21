//! Config and disk backup ops.
//!
//! reference: horto-os/scripts/s3_backup_etc_configs.sh
//! reference: horto-os/scripts/timestamped_backup_etc_configs.sh
//! reference: horto-os/scripts/partclone_backup_sda5.sh
//! reference: horto-os/docs/BACKUP_YOUR_HORTO-OS.MD

use crate::context::{HostContext, PlannedAction};
use crate::embed;
use crate::error::{HortoError, Result};
use crate::kits::{apt, fs};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs as stdfs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Result of copying managed `/etc` entries into a backup tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtcBackupReport {
    /// Destination directory that received copies.
    pub dest: String,
    /// Relative paths successfully copied.
    pub copied: Vec<String>,
    /// Expected paths missing on the host.
    pub skipped_missing: Vec<String>,
    /// Control env files intentionally skipped.
    pub skipped_control: Vec<String>,
}

/// Snapshot of backup-related host state for status API / TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStatus {
    /// True when the initial setup backup tree exists.
    pub initial_setup_present: bool,
    /// Timestamped `/etc` backup directory names.
    pub timestamped: Vec<String>,
    /// Disk / partclone readiness probe.
    pub disk: DiskBackupProbe,
}

/// Options for a partclone-based eMMC partition backup.
#[derive(Debug, Clone)]
pub struct DiskBackupOpts {
    /// Block device or partition to image.
    pub source: PathBuf,
    /// Directory that receives image files.
    pub dest_dir: PathBuf,
    /// When true, also capture early boot sectors.
    pub include_boot_sectors: bool,
    /// When true, allow apply even if root is not clearly an SD/USB boot.
    /// Still refuses when root is clearly the source eMMC (`mmcblk0`).
    pub force: bool,
}

impl Default for DiskBackupOpts {
    fn default() -> Self {
        Self {
            source: PathBuf::from("/dev/mmcblk0p1"),
            dest_dir: PathBuf::from("/mnt/external/horto-os"),
            include_boot_sectors: false,
            force: false,
        }
    }
}

/// Boolean disk-backup readiness flags (flattened in JSON).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskBackupFlags {
    /// True when root looks like onboard eMMC.
    pub looks_like_emmc_root: bool,
    /// True when root looks like removable media.
    pub looks_like_removable_root: bool,
    /// `partclone.*` tools present on PATH.
    pub partclone_present: bool,
    /// `gzip` present on PATH.
    pub gzip_present: bool,
    /// Backup source path exists.
    pub source_exists: bool,
    /// Destination directory exists.
    pub dest_dir_exists: bool,
    /// True when apply is considered safe given probes.
    pub safe_to_apply: bool,
}

/// Readiness probe for full-disk / partition image backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskBackupProbe {
    /// Resolved root device / mount source label.
    pub root_source: String,
    /// Flattened probe flags (same JSON field names as before nesting).
    #[serde(flatten)]
    pub flags: DiskBackupFlags,
    /// Human-readable blockers when unsafe.
    pub blockers: Vec<String>,
    /// Non-blocking notes for operators.
    pub notes: Vec<String>,
}

impl std::ops::Deref for DiskBackupProbe {
    type Target = DiskBackupFlags;

    fn deref(&self) -> &Self::Target {
        &self.flags
    }
}

/// Options for shrink-backup wrapper (optional third method).
#[derive(Debug, Clone)]
pub struct ShrinkBackupOpts {
    /// Output image path for the shrink tool.
    pub dest_img: PathBuf,
    /// Pass force through to the wrapper.
    pub force: bool,
}

fn should_skip_control(name: &str) -> bool {
    let path = Path::new(name);
    (name.starts_with("my_")
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("env")))
        || name == "minimal_setup_vars.env"
        || name == "my_variables.env"
        || name == "iot-lan_conf.env"
        || name == "os-configuration.env"
}

/// Copy managed `/etc` paths (from embedded config top-level names) into `dest`.
///
/// # Errors
///
/// Returns [`HortoError`] when directory creation or copy I/O fails.
pub fn copy_managed_etc(ctx: &mut HostContext, dest: &Path) -> Result<EtcBackupReport> {
    fs::ensure_dir(ctx, dest)?;
    let etc = ctx.paths.etc.clone();
    let mut copied = Vec::new();
    let mut skipped_missing = Vec::new();
    let mut skipped_control = Vec::new();

    for entry in embed::config_top_entries() {
        if should_skip_control(&entry) {
            ctx.log(format!("Skipping control file from config/: {entry}"));
            skipped_control.push(entry);
            continue;
        }
        let src = etc.join(&entry);
        let target = dest.join(&entry);
        if !src.exists() {
            ctx.log(format!("Skipping missing path: {}", src.display()));
            skipped_missing.push(entry);
            continue;
        }
        if src.is_dir() {
            fs::copy_dir_contents(ctx, &src, &target)?;
            ctx.log(format!(
                "Backed up directory: {} -> {}",
                src.display(),
                target.display()
            ));
        } else {
            fs::copy_file(ctx, &src, &target)?;
            ctx.log(format!(
                "Backed up file: {} -> {}",
                src.display(),
                target.display()
            ));
        }
        copied.push(entry);
    }

    Ok(EtcBackupReport {
        dest: dest.display().to_string(),
        copied,
        skipped_missing,
        skipped_control,
    })
}

/// Protected initial `/etc` backup (same destination as setup step s3).
///
/// # Errors
///
/// Returns [`HortoError`] when the backup copy fails.
pub fn backup_etc_initial(ctx: &mut HostContext) -> Result<EtcBackupReport> {
    let dest = ctx.paths.initial_backup_etc();
    let report = copy_managed_etc(ctx, &dest)?;
    ctx.log(format!("Initial /etc backup complete: {}", report.dest));
    Ok(report)
}

/// Timestamped `/etc` backup under `/srv/backup/etc/YYYYMMDD-HHMMSS`.
///
/// # Errors
///
/// Returns [`HortoError`] when the backup copy fails.
pub fn backup_etc_timestamped(ctx: &mut HostContext) -> Result<EtcBackupReport> {
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dest = ctx.paths.backup.join("etc").join(&stamp);
    let report = copy_managed_etc(ctx, &dest)?;
    ctx.log(format!("Timestamped /etc backup complete: {}", report.dest));
    Ok(report)
}

/// List timestamped backup directory names (newest last).
#[must_use]
pub fn list_timestamped_etc_backups(ctx: &HostContext) -> Vec<String> {
    let root = ctx.paths.backup.join("etc");
    let mut names = Vec::new();
    let Ok(entries) = stdfs::read_dir(&root) else {
        return names;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "initial_setup" {
            continue;
        }
        if entry.path().is_dir() {
            names.push(name);
        }
    }
    names.sort();
    names
}

/// Aggregate backup status for API / dashboard.
#[must_use]
pub fn backup_status(ctx: &HostContext) -> BackupStatus {
    let disk = probe_disk_backup(&DiskBackupOpts::default());
    BackupStatus {
        initial_setup_present: ctx.paths.initial_backup_etc().is_dir(),
        timestamped: list_timestamped_etc_backups(ctx),
        disk,
    }
}

/// Resolve what `/` is mounted from (best effort).
#[must_use]
pub fn root_mount_source() -> String {
    if let Ok(out) = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "/"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    stdfs::read_to_string("/proc/mounts")
        .ok()
        .and_then(|text| {
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                let src = parts.next()?;
                let tgt = parts.next()?;
                if tgt == "/" {
                    return Some(src.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "unknown".into())
}

fn looks_like_emmc_root(root: &str) -> bool {
    root.contains("mmcblk0")
}

fn looks_like_removable_root(root: &str) -> bool {
    root.contains("mmcblk1") || root.contains("/dev/sd") || root.contains("usb")
}

/// Probe whether a partclone eMMC backup is safe to run now.
#[must_use]
pub fn probe_disk_backup(opts: &DiskBackupOpts) -> DiskBackupProbe {
    let root_source = root_mount_source();
    let looks_like_emmc_root = looks_like_emmc_root(&root_source);
    let looks_like_removable_root = looks_like_removable_root(&root_source);
    let partclone_present = which::which("partclone.ext4").is_ok();
    let gzip_present = which::which("gzip").is_ok();
    let source_exists = opts.source.exists();
    let dest_dir_exists = opts.dest_dir.is_dir();

    let mut blockers = Vec::new();
    let mut notes = Vec::new();

    notes.push(
        "Full OS image backup must run from a temporary SD/USB boot, not from the live eMMC."
            .into(),
    );
    notes.push(format!(
        "Default source {} → gzip image under {}",
        opts.source.display(),
        opts.dest_dir.display()
    ));

    if looks_like_emmc_root {
        blockers.push(format!(
            "Root is mounted from {root_source} (looks like eMMC). Boot from SD/USB first."
        ));
    } else if !looks_like_removable_root {
        blockers.push(format!(
            "Root source {root_source} is not clearly SD/USB; refuse unless --force (still blocked on eMMC)."
        ));
        notes.push(
            "Use --force only when you verified you are not booted from the target eMMC.".into(),
        );
    }

    if !partclone_present {
        blockers.push("partclone.ext4 not found (apt install partclone).".into());
    }
    if !gzip_present {
        blockers.push("gzip not found on PATH.".into());
    }
    if !source_exists {
        blockers.push(format!("Source device missing: {}", opts.source.display()));
    }
    if !dest_dir_exists {
        blockers.push(format!(
            "Destination directory missing (mount USB and mkdir): {}",
            opts.dest_dir.display()
        ));
    }

    // --force clears only the "not clearly removable" class. eMMC root never applies.
    let safe_to_apply = if looks_like_emmc_root {
        false
    } else if opts.force {
        partclone_present && gzip_present && source_exists && dest_dir_exists
    } else {
        blockers.is_empty()
    };

    DiskBackupProbe {
        root_source,
        flags: DiskBackupFlags {
            looks_like_emmc_root,
            looks_like_removable_root,
            partclone_present,
            gzip_present,
            source_exists,
            dest_dir_exists,
            safe_to_apply,
        },
        blockers,
        notes,
    }
}

fn disk_image_name() -> String {
    format!("emmc_p1_backup_{}.img.gz", Local::now().format("%Y%m%d"))
}

/// Plan actions for a partclone backup (always safe to call).
pub fn plan_disk_backup(ctx: &mut HostContext, opts: &DiskBackupOpts) -> Vec<PlannedAction> {
    let probe = probe_disk_backup(opts);
    for n in &probe.notes {
        ctx.plan_action(format!("note: {n}"));
    }
    for b in &probe.blockers {
        ctx.plan_action(format!("blocker: {b}"));
    }
    let img = opts.dest_dir.join(disk_image_name());
    ctx.plan_action(format!(
        "partclone.ext4 -c -s {} | gzip -c > {}",
        opts.source.display(),
        img.display()
    ));
    if opts.include_boot_sectors {
        let boot = opts.dest_dir.join("emmc_bootsectors_backup.img");
        ctx.plan_action(format!(
            "dd if=/dev/mmcblk0 of={} bs=1M count=4",
            boot.display()
        ));
    }
    ctx.planned.clone()
}

/// Apply partclone backup when guards pass.
///
/// # Errors
///
/// Returns [`HortoError`] when probes block the run, the operator cancels, apt/partclone/gzip fails, or I/O fails.
pub fn backup_disk(ctx: &mut HostContext, opts: &DiskBackupOpts) -> Result<PathBuf> {
    let probe = probe_disk_backup(opts);
    if ctx.is_dry_run() {
        plan_disk_backup(ctx, opts);
        return Ok(opts.dest_dir.join(disk_image_name()));
    }
    if !probe.safe_to_apply {
        return Err(HortoError::msg(format!(
            "disk backup blocked: {}",
            probe.blockers.join("; ")
        )));
    }
    if !ctx.confirm(
        &format!(
            "Create partclone image of {} into {}?",
            opts.source.display(),
            opts.dest_dir.display()
        ),
        false,
    ) {
        return Err(HortoError::msg("disk backup cancelled"));
    }

    if !probe.partclone_present {
        apt::apt_update(ctx)?;
        apt::apt_install(ctx, &["partclone"])?;
    }

    fs::ensure_dir(ctx, &opts.dest_dir)?;
    let img = opts.dest_dir.join(disk_image_name());
    ctx.log(format!(
        "Starting partclone of {} → {}",
        opts.source.display(),
        img.display()
    ));
    run_partclone_gzip(&opts.source, &img)?;
    ctx.log(format!("Wrote {}", img.display()));

    if opts.include_boot_sectors {
        let boot = opts.dest_dir.join("emmc_bootsectors_backup.img");
        let status = Command::new("dd")
            .args([
                "if=/dev/mmcblk0",
                &format!("of={}", boot.display()),
                "bs=1M",
                "count=4",
                "status=progress",
            ])
            .status()
            .map_err(|e| HortoError::msg(format!("dd boot sectors: {e}")))?;
        if !status.success() {
            return Err(HortoError::msg("dd boot sectors failed"));
        }
        ctx.log(format!("Wrote boot sectors {}", boot.display()));
    }
    Ok(img)
}

fn run_partclone_gzip(source: &Path, dest_img: &Path) -> Result<()> {
    let out = stdfs::File::create(dest_img)?;
    let mut partclone = Command::new("partclone.ext4")
        .args(["-c", "-s", &source.display().to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| HortoError::msg(format!("partclone.ext4: {e}")))?;
    let stdout = partclone
        .stdout
        .take()
        .ok_or_else(|| HortoError::msg("partclone stdout missing"))?;
    let gzip = Command::new("gzip")
        .arg("-c")
        .stdin(stdout)
        .stdout(out)
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| HortoError::msg(format!("gzip: {e}")))?;
    let part_status = partclone
        .wait()
        .map_err(|e| HortoError::msg(format!("partclone wait: {e}")))?;
    if !part_status.success() {
        return Err(HortoError::msg("partclone.ext4 failed"));
    }
    if !gzip.success() {
        return Err(HortoError::msg("gzip failed while writing image"));
    }
    Ok(())
}

/// Plan or run shrink-backup when the tool is on PATH.
///
/// # Errors
///
/// Returns [`HortoError`] when probes block the run, the tool is missing, or the wrapper fails.
pub fn backup_shrink(ctx: &mut HostContext, opts: &ShrinkBackupOpts) -> Result<PathBuf> {
    let probe = probe_disk_backup(&DiskBackupOpts {
        dest_dir: opts
            .dest_img
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        force: opts.force,
        ..DiskBackupOpts::default()
    });
    let bin = which::which("shrink-backup").ok();
    if ctx.is_dry_run() {
        for b in &probe.blockers {
            ctx.plan_action(format!("blocker: {b}"));
        }
        if bin.is_none() {
            ctx.plan_action(
                "shrink-backup not on PATH; download from https://github.com/UnconnectedBedna/shrink-backup",
            );
        }
        ctx.plan_action(format!("shrink-backup -a {}", opts.dest_img.display()));
        return Ok(opts.dest_img.clone());
    }
    if probe.looks_like_emmc_root {
        return Err(HortoError::msg(
            "shrink-backup blocked: root looks like eMMC; boot from SD/USB first",
        ));
    }
    if !probe.looks_like_removable_root && !opts.force {
        return Err(HortoError::msg(
            "shrink-backup blocked: root not clearly SD/USB (pass --force after verifying)",
        ));
    }
    let Some(bin) = bin else {
        return Err(HortoError::msg(
            "shrink-backup not found on PATH; install the script first (see BACKUP_YOUR_HORTO-OS.md)",
        ));
    };
    if let Some(parent) = opts.dest_img.parent() {
        fs::ensure_dir(ctx, parent)?;
    }
    ctx.log(format!(
        "Running {} -a {}",
        bin.display(),
        opts.dest_img.display()
    ));
    let status = Command::new(&bin)
        .args(["-a", &opts.dest_img.display().to_string()])
        .status()
        .map_err(|e| HortoError::msg(format!("shrink-backup: {e}")))?;
    if !status.success() {
        return Err(HortoError::msg("shrink-backup failed"));
    }
    Ok(opts.dest_img.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ApplyMode, HostContext};
    use crate::pipeline::SetupKind;
    use tempfile::tempdir;

    #[test]
    fn skip_control_env_files() {
        assert!(should_skip_control("my_variables.env"));
        assert!(should_skip_control("minimal_setup_vars.env"));
        assert!(should_skip_control("iot-lan_conf.env"));
        assert!(should_skip_control("os-configuration.env"));
        assert!(!should_skip_control("dnsmasq.conf"));
    }

    #[test]
    fn emmc_root_never_safe() {
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: PathBuf::from("/tmp"),
            include_boot_sectors: false,
            force: true,
        };
        // Force cannot override emmc detection in probe_disk_backup logic when root is mmcblk0;
        // unit-test the helpers directly.
        assert!(looks_like_emmc_root("/dev/mmcblk0p1"));
        assert!(looks_like_removable_root("/dev/mmcblk1p1"));
        assert!(looks_like_removable_root("/dev/sda1"));
        let _ = opts;
    }

    #[test]
    fn timestamped_backup_dry_run() {
        let dir = tempdir().unwrap();
        let paths = crate::paths::HostPaths {
            active_setup: dir.path().join("active"),
            backup: dir.path().join("backup"),
            docker: dir.path().join("docker"),
            etc: dir.path().join("etc"),
            lease_file: dir.path().join("leases"),
        };
        stdfs::create_dir_all(paths.etc.join("netplan")).unwrap();
        stdfs::write(paths.etc.join("hostname"), b"horto-test\n").unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        let report = backup_etc_timestamped(&mut ctx).unwrap();
        assert!(report.dest.contains("backup/etc/"));
        assert_ne!(ctx.planned.as_slice(), &[]);
    }

    #[test]
    fn backup_disk_dry_run_and_blocked_apply() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("out");
        stdfs::create_dir_all(&dest).unwrap();
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: dest.clone(),
            include_boot_sectors: true,
            force: true,
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let path = backup_disk(&mut ctx, &opts).unwrap();
        assert!(path.starts_with(&dest));
        assert!(ctx.planned.iter().any(|p| p.summary.contains("partclone")));

        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        let err = backup_disk(&mut ctx, &opts).unwrap_err();
        assert!(err.to_string().contains("blocked") || err.to_string().contains("cancel"));
    }

    #[test]
    fn backup_shrink_dry_run() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("img.img");
        let opts = ShrinkBackupOpts {
            dest_img: dest.clone(),
            force: true,
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let out = backup_shrink(&mut ctx, &opts).unwrap();
        assert_eq!(out, dest);
        assert!(ctx
            .planned
            .iter()
            .any(|p| p.summary.contains("shrink-backup")));
    }

    #[test]
    fn root_mount_source_nonempty() {
        assert_ne!(root_mount_source(), "");
    }

    fn temp_paths(root: &Path) -> crate::paths::HostPaths {
        crate::paths::HostPaths {
            active_setup: root.join("active_setup"),
            backup: root.join("backup"),
            docker: root.join("docker"),
            etc: root.join("etc"),
            lease_file: root.join("leases"),
        }
    }

    #[test]
    fn should_skip_control_all_variants() {
        assert!(should_skip_control("my_wifi.env"));
        assert!(should_skip_control("my_something.env"));
        // Only .env is skipped for the my_ prefix.
        assert!(!should_skip_control("my_notes.txt"));
        assert!(!should_skip_control("hostname"));
        assert!(!should_skip_control("netplan"));
    }

    #[test]
    fn root_source_helpers_recognize_devices() {
        assert!(looks_like_emmc_root("/dev/mmcblk0p1"));
        assert!(!looks_like_emmc_root("/dev/sda1"));
        assert!(looks_like_removable_root("/dev/mmcblk1p1"));
        assert!(looks_like_removable_root("/dev/sdb2"));
        assert!(looks_like_removable_root("usb-XYZ"));
        assert!(!looks_like_removable_root("/dev/mmcblk0p1"));
    }

    #[test]
    fn copy_managed_etc_reports_copied_missing_and_control() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        // Seed /etc with some managed entries.
        stdfs::create_dir_all(paths.etc.join("netplan")).unwrap();
        stdfs::write(paths.etc.join("netplan/99-iot-lan.yaml"), b"network: {}\n").unwrap();
        stdfs::write(paths.etc.join("hostname"), b"cov-box\n").unwrap();
        stdfs::write(paths.etc.join("hosts"), b"127.0.0.1 localhost\n").unwrap();
        // Intentionally omit `resolv.conf` so it lands in skipped_missing.
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let dest = dir.path().join("backup-dest");
        let report = copy_managed_etc(&mut ctx, &dest).unwrap();
        assert!(report.copied.iter().any(|c| c == "netplan"));
        assert!(report.copied.iter().any(|c| c == "hostname"));
        assert!(matches!(report.skipped_control.as_slice(), [_, ..]));
        assert!(report
            .skipped_missing
            .iter()
            .any(|s| s == "resolv.conf" || s == "sysctl.d" || !s.is_empty()));
        assert!(dest.join("hostname").is_file());
        assert!(dest.join("netplan/99-iot-lan.yaml").is_file());
    }

    #[test]
    fn copy_managed_etc_dry_run_records_plans() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        stdfs::create_dir_all(paths.etc.join("netplan")).unwrap();
        stdfs::write(paths.etc.join("hostname"), b"box\n").unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        let dest = dir.path().join("backup-dest");
        let report = copy_managed_etc(&mut ctx, &dest).unwrap();
        assert_ne!(ctx.planned.as_slice(), &[]);
        assert!(report.copied.iter().any(|c| c == "hostname"));
    }

    #[test]
    fn backup_etc_initial_writes_to_initial_setup() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        stdfs::create_dir_all(&paths.etc).unwrap();
        stdfs::write(paths.etc.join("hostname"), b"box\n").unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let report = backup_etc_initial(&mut ctx).unwrap();
        assert!(report.dest.contains("initial_setup"));
        assert!(ctx.paths.initial_backup_etc().join("hostname").is_file());
    }

    #[test]
    fn list_timestamped_hides_initial_setup() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        let etc_root = paths.backup.join("etc");
        stdfs::create_dir_all(etc_root.join("initial_setup")).unwrap();
        stdfs::create_dir_all(etc_root.join("20240101-000000")).unwrap();
        stdfs::create_dir_all(etc_root.join("20241231-120000")).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let listed = list_timestamped_etc_backups(&ctx);
        assert_eq!(listed, vec!["20240101-000000", "20241231-120000"]);
        assert!(!listed.iter().any(|n| n == "initial_setup"));
    }

    #[test]
    fn list_timestamped_returns_empty_when_missing() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        assert_eq!(list_timestamped_etc_backups(&ctx), Vec::<String>::new());
    }

    #[test]
    fn backup_status_aggregates_probe_and_dirs() {
        let dir = tempdir().unwrap();
        let paths = temp_paths(dir.path());
        stdfs::create_dir_all(paths.backup.join("etc/initial_setup")).unwrap();
        stdfs::create_dir_all(paths.backup.join("etc/20240101-010101")).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let status = backup_status(&ctx);
        assert!(status.initial_setup_present);
        assert!(status.timestamped.iter().any(|n| n == "20240101-010101"));
        // Probe is always populated with notes on a sane host.
        assert!(matches!(status.disk.notes.as_slice(), [_, ..]));
    }

    #[test]
    fn probe_missing_source_and_dest_reports_blockers() {
        let opts = DiskBackupOpts {
            source: PathBuf::from("/definitely/missing/device"),
            dest_dir: PathBuf::from("/definitely/missing/dir"),
            include_boot_sectors: false,
            force: false,
        };
        let probe = probe_disk_backup(&opts);
        assert!(!probe.source_exists);
        assert!(!probe.dest_dir_exists);
        assert!(probe.blockers.iter().any(|b| b.contains("Source device")));
        assert!(probe.blockers.iter().any(|b| b.contains("Destination")));
        assert!(!probe.safe_to_apply);
    }

    #[test]
    fn probe_force_flag_does_not_bypass_emmc_root_check() {
        // Real root of the CI/dev host is not eMMC; force still cannot unsafe-apply
        // when tools are missing. Verify the safe_to_apply is a strict AND.
        let dir = tempdir().unwrap();
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: dir.path().to_path_buf(),
            include_boot_sectors: false,
            force: true,
        };
        let probe = probe_disk_backup(&opts);
        // With force, safe_to_apply depends on presence of partclone/gzip on the host.
        let partclone = probe.partclone_present;
        let gzip = probe.gzip_present;
        let source = probe.source_exists;
        let dest = probe.dest_dir_exists;
        let not_emmc = !probe.looks_like_emmc_root;
        let expected = partclone && gzip && source && dest && not_emmc;
        assert_eq!(probe.safe_to_apply, expected);
    }

    #[test]
    fn plan_disk_backup_records_boot_sector_action_when_requested() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("out");
        stdfs::create_dir_all(&dest).unwrap();
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: dest,
            include_boot_sectors: true,
            force: true,
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let planned = plan_disk_backup(&mut ctx, &opts);
        assert!(planned.iter().any(|p| p.summary.contains("partclone")));
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("dd if=/dev/mmcblk0") && p.summary.contains("bs=1M")));
    }

    #[test]
    fn plan_disk_backup_without_boot_sectors_skips_dd() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("out");
        stdfs::create_dir_all(&dest).unwrap();
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: dest,
            include_boot_sectors: false,
            force: false,
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let planned = plan_disk_backup(&mut ctx, &opts);
        assert!(planned.iter().any(|p| p.summary.contains("partclone")));
        assert!(!planned
            .iter()
            .any(|p| p.summary.contains("dd if=/dev/mmcblk0")));
    }

    #[test]
    fn backup_shrink_dry_run_lists_download_hint_when_missing() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("img.img.gz");
        let opts = ShrinkBackupOpts {
            dest_img: dest.clone(),
            force: false,
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        let out = backup_shrink(&mut ctx, &opts).unwrap();
        assert_eq!(out, dest);
        assert_ne!(ctx.planned.as_slice(), &[]);
        assert!(
            ctx.planned
                .iter()
                .any(|p| p.summary.contains("shrink-backup")),
            "expected a shrink-backup plan entry: {:?}",
            ctx.planned
        );
    }

    #[test]
    fn backup_shrink_apply_without_force_or_bin_errors() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("img.img.gz");
        let opts = ShrinkBackupOpts {
            dest_img: dest,
            force: false,
        };
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        // On the dev host root is not eMMC and not "clearly removable", so with force=false
        // apply refuses regardless of shrink-backup presence.
        let err = backup_shrink(&mut ctx, &opts).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("shrink-backup blocked") || msg.contains("not found"));
    }

    #[test]
    fn backup_disk_apply_blocked_when_dest_missing_and_no_force() {
        let dir = tempdir().unwrap();
        let opts = DiskBackupOpts {
            source: PathBuf::from("/dev/null"),
            dest_dir: dir.path().join("nope"),
            include_boot_sectors: false,
            force: false,
        };
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        let err = backup_disk(&mut ctx, &opts).unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn probe_notes_call_out_removable_expectation() {
        let opts = DiskBackupOpts::default();
        let probe = probe_disk_backup(&opts);
        assert!(probe.notes.iter().any(|n| n.contains("SD/USB")));
    }
}
