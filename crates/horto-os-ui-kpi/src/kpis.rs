//! Derive numeric KPI tiles from status API payloads (no GPUI).

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KpiTone {
    Ok,
    Warn,
    Bad,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KpiTile {
    pub id: &'static str,
    pub label: &'static str,
    pub value: String,
    pub tone: KpiTone,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct ContainerInfo {
    #[serde(default)]
    pub names: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct UrlInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub up: bool,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct StepStatusRow {
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct SetupStatusReport {
    #[serde(default)]
    pub steps: Vec<StepStatusRow>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct DoctorReport {
    #[serde(default)]
    pub is_root: bool,
    #[serde(default)]
    pub has_sudo: bool,
    #[serde(default)]
    pub docker_present: bool,
    #[serde(default)]
    pub active_setup_dir: bool,
    #[serde(default)]
    pub full_env: bool,
    #[serde(default)]
    pub minimal_env: bool,
    #[serde(default)]
    pub docker_dir: bool,
    #[serde(default)]
    pub backup_dir: bool,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct LeaseEntry {
    #[serde(default)]
    pub hostname: String,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct DiskBackupProbe {
    #[serde(default)]
    pub safe_to_apply: bool,
    #[serde(default)]
    pub blockers: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct BackupStatus {
    #[serde(default)]
    pub initial_setup_present: bool,
    #[serde(default)]
    pub timestamped: Vec<String>,
    #[serde(default)]
    pub disk: DiskBackupProbe,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct BoxStatus {
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub setup: SetupStatusReport,
    #[serde(default)]
    pub doctor: DoctorReport,
    #[serde(default)]
    pub backup: BackupStatus,
    #[serde(default)]
    pub containers: Vec<ContainerInfo>,
    #[serde(default)]
    pub leases: Vec<LeaseEntry>,
    #[serde(default)]
    pub urls: Vec<UrlInfo>,
}

/// Build the KPI tile set from health + optional status body.
#[must_use]
pub fn derive_kpis(health_ok: Option<bool>, status: Option<&BoxStatus>) -> Vec<KpiTile> {
    let mut tiles = Vec::with_capacity(9);

    tiles.push(match health_ok {
        Some(true) => tile("api", "API", "ok", KpiTone::Ok),
        Some(false) => tile("api", "API", "down", KpiTone::Bad),
        None => tile("api", "API", "unknown", KpiTone::Neutral),
    });

    let Some(st) = status else {
        for (id, label) in [
            ("containers", "Containers"),
            ("services", "Services"),
            ("setup", "Setup"),
            ("doctor", "Doctor"),
            ("leases", "DHCP leases"),
            ("backup_snaps", "Backup snaps"),
            ("backup_disk", "Backup disk"),
            ("backup_initial", "Initial backup"),
        ] {
            tiles.push(tile(id, label, "-", KpiTone::Neutral));
        }
        return tiles;
    };

    let total_c = st.containers.len();
    let up_c = st
        .containers
        .iter()
        .filter(|c| container_looks_up(&c.status))
        .count();
    tiles.push(tile(
        "containers",
        "Containers",
        format!("{up_c}/{total_c}"),
        ratio_tone(up_c, total_c),
    ));

    let total_s = st.urls.len();
    let up_s = st.urls.iter().filter(|u| u.up).count();
    tiles.push(tile(
        "services",
        "Services",
        format!("{up_s}/{total_s}"),
        ratio_tone(up_s, total_s),
    ));

    let steps = &st.setup.steps;
    let total_setup = steps.len();
    let done = steps.iter().filter(|s| s.status == "done").count();
    let failed = steps.iter().filter(|s| s.status == "failed").count();
    let stale = steps.iter().filter(|s| s.status == "stale").count();
    let setup_value = if failed > 0 || stale > 0 {
        format!("{done}/{total_setup} (!)")
    } else {
        format!("{done}/{total_setup}")
    };
    let setup_tone = if failed > 0 {
        KpiTone::Bad
    } else if stale > 0 || (total_setup > 0 && done < total_setup) {
        KpiTone::Warn
    } else if total_setup == 0 {
        KpiTone::Neutral
    } else {
        KpiTone::Ok
    };
    tiles.push(tile("setup", "Setup", setup_value, setup_tone));

    let checks = doctor_checks(&st.doctor);
    let passed = checks.iter().filter(|&&c| c).count();
    let total_d = checks.len();
    tiles.push(tile(
        "doctor",
        "Doctor",
        format!("{passed}/{total_d}"),
        ratio_tone(passed, total_d),
    ));

    let leases = st.leases.len();
    tiles.push(tile(
        "leases",
        "DHCP leases",
        leases.to_string(),
        if leases == 0 {
            KpiTone::Neutral
        } else {
            KpiTone::Ok
        },
    ));

    let snaps = st.backup.timestamped.len();
    tiles.push(tile(
        "backup_snaps",
        "Backup snaps",
        snaps.to_string(),
        if snaps == 0 {
            KpiTone::Warn
        } else {
            KpiTone::Ok
        },
    ));

    let blockers = st.backup.disk.blockers.len();
    let (disk_value, disk_tone) = if st.backup.disk.safe_to_apply {
        ("safe".into(), KpiTone::Ok)
    } else if blockers > 0 {
        (format!("blocked ({blockers})"), KpiTone::Bad)
    } else {
        ("blocked".into(), KpiTone::Warn)
    };
    tiles.push(tile("backup_disk", "Backup disk", disk_value, disk_tone));

    tiles.push(if st.backup.initial_setup_present {
        tile("backup_initial", "Initial backup", "yes", KpiTone::Ok)
    } else {
        tile("backup_initial", "Initial backup", "no", KpiTone::Warn)
    });

    tiles
}

fn tile(id: &'static str, label: &'static str, value: impl Into<String>, tone: KpiTone) -> KpiTile {
    KpiTile {
        id,
        label,
        value: value.into(),
        tone,
    }
}

fn doctor_checks(d: &DoctorReport) -> [bool; 8] {
    [
        d.is_root,
        d.has_sudo,
        d.docker_present,
        d.active_setup_dir,
        d.full_env,
        d.minimal_env,
        d.docker_dir,
        d.backup_dir,
    ]
}

fn container_looks_up(status: &str) -> bool {
    let s = status.to_ascii_lowercase();
    s.contains("up") && !s.contains("exited") && !s.contains("dead") && !s.contains("created")
}

fn ratio_tone(ok: usize, total: usize) -> KpiTone {
    if total == 0 {
        KpiTone::Neutral
    } else if ok == total {
        KpiTone::Ok
    } else if ok == 0 {
        KpiTone::Bad
    } else {
        KpiTone::Warn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_only_fills_placeholders() {
        let tiles = derive_kpis(Some(true), None);
        assert_eq!(tiles[0].value, "ok");
        assert_eq!(tiles[0].tone, KpiTone::Ok);
        assert_eq!(tiles.len(), 9);
        assert!(tiles.iter().skip(1).all(|t| t.value == "-"));
    }

    #[test]
    fn derives_counts_from_status() {
        let status = BoxStatus {
            hostname: "deb".into(),
            containers: vec![
                ContainerInfo {
                    names: "a".into(),
                    status: "Up 2 hours".into(),
                },
                ContainerInfo {
                    names: "b".into(),
                    status: "Exited (0)".into(),
                },
            ],
            urls: vec![
                UrlInfo {
                    name: "Dockge".into(),
                    up: true,
                },
                UrlInfo {
                    name: "Homepage".into(),
                    up: false,
                },
            ],
            setup: SetupStatusReport {
                steps: vec![
                    StepStatusRow {
                        status: "done".into(),
                    },
                    StepStatusRow {
                        status: "pending".into(),
                    },
                    StepStatusRow {
                        status: "failed".into(),
                    },
                ],
            },
            doctor: DoctorReport {
                is_root: true,
                has_sudo: true,
                docker_present: true,
                active_setup_dir: true,
                full_env: false,
                minimal_env: false,
                docker_dir: true,
                backup_dir: false,
            },
            leases: vec![LeaseEntry {
                hostname: "phone".into(),
            }],
            backup: BackupStatus {
                initial_setup_present: true,
                timestamped: vec!["t1".into(), "t2".into()],
                disk: DiskBackupProbe {
                    safe_to_apply: false,
                    blockers: vec!["root on emmc".into()],
                },
            },
        };
        let tiles = derive_kpis(Some(true), Some(&status));
        let by_id = |id: &str| tiles.iter().find(|t| t.id == id).unwrap();

        assert_eq!(by_id("containers").value, "1/2");
        assert_eq!(by_id("containers").tone, KpiTone::Warn);
        assert_eq!(by_id("services").value, "1/2");
        assert_eq!(by_id("setup").value, "1/3 (!)");
        assert_eq!(by_id("setup").tone, KpiTone::Bad);
        assert_eq!(by_id("doctor").value, "5/8");
        assert_eq!(by_id("leases").value, "1");
        assert_eq!(by_id("backup_snaps").value, "2");
        assert_eq!(by_id("backup_disk").value, "blocked (1)");
        assert_eq!(by_id("backup_disk").tone, KpiTone::Bad);
        assert_eq!(by_id("backup_initial").value, "yes");
    }

    #[test]
    fn parses_partial_json_defaults() {
        let st: BoxStatus = serde_json::from_str(r#"{"hostname":"box"}"#).unwrap();
        assert_eq!(st.hostname, "box");
        assert!(st.containers.is_empty());
        assert!(st.setup.steps.is_empty());
    }
}
