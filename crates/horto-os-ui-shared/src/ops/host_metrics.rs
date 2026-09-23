//! Local host sensors for status API / overview (CPU, load, disk, OS, apt).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Snapshot of box host sensors attached to [`super::status::BoxStatus`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HostMetrics {
    /// Aggregate CPU busy percent from two `/proc/stat` samples (~100 ms apart).
    pub cpu_percent: Option<f32>,
    /// 1-minute load average from `/proc/loadavg`.
    pub load_1: Option<f32>,
    /// 5-minute load average.
    pub load_5: Option<f32>,
    /// 15-minute load average.
    pub load_15: Option<f32>,
    /// Root filesystem total bytes (`statvfs("/")`).
    pub disk_total_bytes: Option<u64>,
    /// Root filesystem used bytes (total minus available).
    pub disk_used_bytes: Option<u64>,
    /// Root filesystem available bytes for non-root.
    pub disk_avail_bytes: Option<u64>,
    /// `PRETTY_NAME` from `/etc/os-release`.
    pub os_pretty_name: Option<String>,
    /// `ID` from `/etc/os-release`.
    pub os_id: Option<String>,
    /// `VERSION_ID` from `/etc/os-release`.
    pub os_version_id: Option<String>,
    /// Armbian `VERSION` when `/etc/armbian-release` exists.
    pub armbian_version: Option<String>,
    /// Armbian board name (`BOARD_NAME` or `BOARD`).
    pub armbian_board: Option<String>,
    /// Kernel release (`uname -r`).
    pub kernel: Option<String>,
    /// Count of upgradable packages from the local apt cache (no `apt update`).
    pub apt_upgradable: Option<u32>,
}

/// Collect host metrics from the local machine.
///
/// Reads `/proc`, os-release, optional Armbian release, root `statvfs`, and
/// `apt list --upgradable` (2 s timeout). Never runs `apt update`.
#[must_use]
pub fn collect_host_metrics() -> HostMetrics {
    let (load_one, load_five, load_fifteen) =
        parse_loadavg(&read_to_string("/proc/loadavg").unwrap_or_default());
    let (disk_total_bytes, disk_used_bytes, disk_avail_bytes) = disk_root();
    let os = parse_os_release(&read_to_string("/etc/os-release").unwrap_or_default());
    let armbian =
        parse_armbian_release(&read_to_string("/etc/armbian-release").unwrap_or_default());
    HostMetrics {
        cpu_percent: cpu_percent_sample(),
        load_1: load_one,
        load_5: load_five,
        load_15: load_fifteen,
        disk_total_bytes,
        disk_used_bytes,
        disk_avail_bytes,
        os_pretty_name: os.pretty_name,
        os_id: os.id,
        os_version_id: os.version_id,
        armbian_version: armbian.version,
        armbian_board: armbian.board,
        kernel: kernel_release(),
        apt_upgradable: apt_upgradable_count(),
    }
}

fn read_to_string(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path.as_ref()).ok()
}

/// Parse the aggregate `cpu` line counters from `/proc/stat` text.
#[must_use]
pub fn parse_proc_stat_cpu(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let mut parts = line.split_whitespace().skip(1);
    let mut values = [0_u64; 10];
    for slot in &mut values {
        *slot = parts.next()?.parse().ok()?;
    }
    // user nice system idle iowait irq softirq steal guest guest_nice
    let idle = values[3].saturating_add(values[4]);
    let total: u64 = values.iter().take(8).sum();
    Some((idle, total))
}

/// CPU busy percent from two idle/total samples.
#[must_use]
pub fn cpu_percent_from_samples(first: (u64, u64), second: (u64, u64)) -> Option<f32> {
    let idle_delta = second.0.saturating_sub(first.0);
    let total_delta = second.1.saturating_sub(first.1);
    if total_delta == 0 {
        return None;
    }
    let busy_pct = total_delta.saturating_sub(idle_delta).saturating_mul(100) / total_delta;
    let capped = busy_pct.min(100);
    let as_u8 = u8::try_from(capped).unwrap_or(100);
    Some(f32::from(as_u8))
}

fn cpu_percent_sample() -> Option<f32> {
    let first = parse_proc_stat_cpu(&read_to_string("/proc/stat")?)?;
    thread::sleep(Duration::from_millis(100));
    let second = parse_proc_stat_cpu(&read_to_string("/proc/stat")?)?;
    cpu_percent_from_samples(first, second)
}

/// Parse `/proc/loadavg` into 1 / 5 / 15 minute averages.
#[must_use]
pub fn parse_loadavg(raw: &str) -> (Option<f32>, Option<f32>, Option<f32>) {
    let mut parts = raw.split_whitespace();
    let a = parts.next().and_then(|s| s.parse().ok());
    let b = parts.next().and_then(|s| s.parse().ok());
    let c = parts.next().and_then(|s| s.parse().ok());
    (a, b, c)
}

/// Parse key=value lines from `/etc/os-release` style text.
#[must_use]
pub fn parse_os_release(raw: &str) -> OsReleaseFields {
    let map = parse_kv(raw);
    OsReleaseFields {
        pretty_name: map_get(&map, "PRETTY_NAME"),
        id: map_get(&map, "ID"),
        version_id: map_get(&map, "VERSION_ID"),
    }
}

/// Public fields from [`parse_os_release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsReleaseFields {
    /// `PRETTY_NAME`.
    pub pretty_name: Option<String>,
    /// `ID`.
    pub id: Option<String>,
    /// `VERSION_ID`.
    pub version_id: Option<String>,
}

/// Parse Armbian release file fields.
#[must_use]
pub fn parse_armbian_release(raw: &str) -> ArmbianReleaseFields {
    let map = parse_kv(raw);
    let board = map_get(&map, "BOARD_NAME").or_else(|| map_get(&map, "BOARD"));
    ArmbianReleaseFields {
        version: map_get(&map, "VERSION"),
        board,
    }
}

/// Public fields from [`parse_armbian_release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmbianReleaseFields {
    /// Armbian `VERSION`.
    pub version: Option<String>,
    /// `BOARD_NAME` or `BOARD`.
    pub board: Option<String>,
}

fn parse_kv(raw: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let mut val = v.trim();
        if val.len() >= 2 && val.starts_with('"') && val.ends_with('"') {
            val = &val[1..val.len() - 1];
        }
        if !key.is_empty() {
            map.insert(key.to_owned(), val.to_owned());
        }
    }
    map
}

fn map_get(map: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn disk_root() -> (Option<u64>, Option<u64>, Option<u64>) {
    disk_statvfs(c"/")
}

fn disk_statvfs(path: &std::ffi::CStr) -> (Option<u64>, Option<u64>, Option<u64>) {
    // SAFETY: path is a valid C string; `statvfs` writes into `buf` on success.
    let mut buf = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let rc = unsafe { libc::statvfs(path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return (None, None, None);
    }
    let st = unsafe { buf.assume_init() };
    let bsize = st.f_frsize;
    let total = bsize.saturating_mul(st.f_blocks);
    let avail = bsize.saturating_mul(st.f_bavail);
    let used = total.saturating_sub(avail);
    (Some(total), Some(used), Some(avail))
}

fn kernel_release() -> Option<String> {
    let o = Command::new("uname").arg("-r").output().ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn apt_upgradable_count() -> Option<u32> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out = Command::new("apt")
            .args(["list", "--upgradable"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(o)) if o.status.success() => {
            Some(count_apt_upgradable(&String::from_utf8_lossy(&o.stdout)))
        }
        _ => None,
    }
}

/// Count package lines in `apt list --upgradable` stdout (skips `Listing...`).
#[must_use]
pub fn count_apt_upgradable(stdout: &str) -> u32 {
    let n = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("Listing"))
        .filter(|line| line.contains('/'))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_cpu_parses_and_delta() {
        let a = "cpu  100 0 50 850 0 0 0 0 0 0\ncpu0 50 0 25 425 0 0 0 0 0 0\n";
        let b = "cpu  120 0 60 870 0 0 0 0 0 0\n";
        let first = parse_proc_stat_cpu(a).expect("first");
        let second = parse_proc_stat_cpu(b).expect("second");
        assert_eq!(first, (850, 1000));
        assert_eq!(second, (870, 1050));
        let pct = cpu_percent_from_samples(first, second).expect("pct");
        // idle +20, total +50 → busy 60%
        assert!((pct - 60.0).abs() < 0.01);
    }

    #[test]
    fn loadavg_parses_three() {
        let (a, b, c) = parse_loadavg("0.30 0.40 0.50 1/234 5678");
        assert_eq!(a, Some(0.3));
        assert_eq!(b, Some(0.4));
        assert_eq!(c, Some(0.5));
    }

    #[test]
    fn os_release_strips_quotes() {
        let raw = r#"
NAME="Debian GNU/Linux"
PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
ID=debian
VERSION_ID="12"
"#;
        let os = parse_os_release(raw);
        assert_eq!(
            os.pretty_name.as_deref(),
            Some("Debian GNU/Linux 12 (bookworm)")
        );
        assert_eq!(os.id.as_deref(), Some("debian"));
        assert_eq!(os.version_id.as_deref(), Some("12"));
    }

    #[test]
    fn armbian_prefers_board_name() {
        let raw = "VERSION=24.11.1\nBOARD=orangepi5\nBOARD_NAME=\"Orange Pi 5\"\n";
        let a = parse_armbian_release(raw);
        assert_eq!(a.version.as_deref(), Some("24.11.1"));
        assert_eq!(a.board.as_deref(), Some("Orange Pi 5"));
    }

    #[test]
    fn apt_list_counts_packages() {
        let stdout = "Listing... Done\nlibc6/stable 2.36-9 amd64 [upgradable from: 2.36-8]\nbash/stable 5.2-1 amd64 [upgradable from: 5.1-2]\n";
        assert_eq!(count_apt_upgradable(stdout), 2);
        assert_eq!(count_apt_upgradable("Listing...\n"), 0);
    }
}
