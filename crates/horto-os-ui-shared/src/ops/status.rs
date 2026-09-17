use crate::context::HostContext;
use crate::kits::docker::{self, ContainerInfo};
use crate::ops::backup::{self, BackupStatus};
use crate::ops::doctor::{self, DoctorReport};
use crate::ops::leases::{self, LeaseEntry};
use crate::pipeline::{self, SetupKind};
use crate::resume::{self, StepStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlInfo {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStatusRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub step_version: u32,
    pub destructive: bool,
    pub needs_reboot_after: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStatusReport {
    pub kind: String,
    pub steps: Vec<StepStatusRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxStatus {
    pub hostname: String,
    pub setup: SetupStatusReport,
    pub doctor: DoctorReport,
    pub backup: BackupStatus,
    pub containers: Vec<ContainerInfo>,
    pub leases: Vec<LeaseEntry>,
    pub urls: Vec<UrlInfo>,
}

pub fn default_urls() -> Vec<UrlInfo> {
    vec![
        UrlInfo {
            name: "Homepage".into(),
            url: "http://localhost:3021".into(),
        },
        UrlInfo {
            name: "Dockge".into(),
            url: "http://localhost:5001".into(),
        },
        UrlInfo {
            name: "Cockpit".into(),
            url: "http://localhost:9090".into(),
        },
    ]
}

pub fn setup_status(ctx: &HostContext, kind: SetupKind) -> SetupStatusReport {
    let state = resume::load(&ctx.paths.resume_file()).unwrap_or_default();
    let steps = pipeline::pipeline(kind)
        .iter()
        .map(|s| {
            let st = resume::status_for(&state, *s);
            let status = match st {
                StepStatus::Pending => {
                    if s.is_done(ctx) {
                        "done".to_string()
                    } else {
                        "pending".to_string()
                    }
                }
                StepStatus::Done => "done".into(),
                StepStatus::Stale => "stale".into(),
                StepStatus::Failed => "failed".into(),
            };
            StepStatusRow {
                id: s.id().into(),
                title: s.title().into(),
                status,
                step_version: s.step_version(),
                destructive: s.destructive(),
                needs_reboot_after: s.needs_reboot_after(),
            }
        })
        .collect();
    SetupStatusReport {
        kind: kind.as_str().into(),
        steps,
    }
}

pub fn box_status(ctx: &HostContext, kind: SetupKind) -> BoxStatus {
    let hostname = fs_hostname().unwrap_or_else(|| "unknown".into());
    BoxStatus {
        hostname,
        setup: setup_status(ctx, kind),
        doctor: doctor::doctor(ctx),
        backup: backup::backup_status(ctx),
        containers: docker::list_containers().unwrap_or_default(),
        leases: leases::read_leases(&ctx.paths.lease_file, &ctx.paths.leases_json()),
        urls: default_urls(),
    }
}

fn fs_hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| hostname_cmd().or_else(|| std::env::var("HOSTNAME").ok()))
}

fn hostname_cmd() -> Option<String> {
    let o = std::process::Command::new("hostname").output().ok()?;
    if o.status.success() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}
