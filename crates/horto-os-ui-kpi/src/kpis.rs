//! Derive live metric samples from status API payloads (no GPUI).

use crate::history::MetricSample;
use serde::Deserialize;

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
    pub url: String,
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

/// Build a numeric sample for chart history from status (+ optional EVCC watts).
#[must_use]
pub fn sample_metrics(
    status: Option<&BoxStatus>,
    pv_w: Option<f32>,
    grid_w: Option<f32>,
    home_w: Option<f32>,
    charge_w: Option<f32>,
) -> MetricSample {
    let Some(st) = status else {
        return MetricSample {
            pv_w,
            grid_w,
            home_w,
            charge_w,
            ..MetricSample::default()
        };
    };

    let containers_total = st.containers.len() as f32;
    let containers_up = st
        .containers
        .iter()
        .filter(|c| container_looks_up(&c.status))
        .count() as f32;
    let services_total = st.urls.len() as f32;
    let services_up = st.urls.iter().filter(|u| u.up).count() as f32;

    let steps = &st.setup.steps;
    let total_setup = steps.len() as f32;
    let done = steps.iter().filter(|s| s.status == "done").count() as f32;
    let setup_done_pct = if total_setup > 0.0 {
        100.0 * done / total_setup
    } else {
        0.0
    };

    let checks = doctor_checks(&st.doctor);
    let passed = checks.iter().filter(|&&c| c).count() as f32;
    let doctor_pct = 100.0 * passed / checks.len() as f32;

    MetricSample {
        containers_up,
        containers_total,
        services_up,
        services_total,
        setup_done_pct,
        doctor_pct,
        leases: st.leases.len() as f32,
        pv_w,
        grid_w,
        home_w,
        charge_w,
    }
}

/// First service URL whose name normalizes to `evcc`, if any.
#[must_use]
pub fn evcc_base_url(status: Option<&BoxStatus>) -> Option<String> {
    status?
        .urls
        .iter()
        .find(|u| normalize_key(&u.name) == "evcc")
        .map(|u| u.url.trim_end_matches('/').to_owned())
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

fn normalize_key(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_fleet_counts() {
        let status = BoxStatus {
            containers: vec![
                ContainerInfo {
                    names: "a".into(),
                    status: "Up 1h".into(),
                },
                ContainerInfo {
                    names: "b".into(),
                    status: "Exited".into(),
                },
            ],
            urls: vec![
                UrlInfo {
                    name: "EVCC".into(),
                    url: "http://box:7070".into(),
                    up: true,
                },
                UrlInfo {
                    name: "Dockge".into(),
                    url: "http://box:5001".into(),
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
            ..BoxStatus::default()
        };
        let s = sample_metrics(Some(&status), Some(1200.0), Some(-300.0), Some(900.0), None);
        assert!((s.containers_up - 1.0).abs() < f32::EPSILON);
        assert!((s.services_up - 1.0).abs() < f32::EPSILON);
        assert!((s.setup_done_pct - 50.0).abs() < f32::EPSILON);
        assert!((s.doctor_pct - 62.5).abs() < 0.1);
        assert_eq!(
            evcc_base_url(Some(&status)).as_deref(),
            Some("http://box:7070")
        );
    }
}
