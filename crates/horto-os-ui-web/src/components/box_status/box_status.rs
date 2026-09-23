use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::busy::spawn_busy;
use crate::status::{post_backup_etc, BoxStatus, HostMetrics};

include!(concat!(env!("OUT_DIR"), "/rangular/box_status_view.rs"));

#[component]
pub fn BoxStatusPanel(
    status: BoxStatus,
    api_base: String,
    token: RwSignal<String>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let hostname = status.hostname.clone();
    let container_count = status.containers.len().to_string();
    let backup_label = if status.backup.initial_setup_present {
        "present"
    } else {
        "missing"
    }
    .to_string();
    let backup_count = status.backup.timestamped.len().to_string();
    let cpu_label = format_cpu(&status.host);
    let disk_label = format_disk(&status.host);
    let os_label = format_os(&status.host);
    let updates_label = format_updates(&status.host);

    let confirm_open = RwSignal::new(false);
    let backup_busy = RwSignal::new(false);
    let backup_msg = RwSignal::new(String::new());
    let api_for_backup = StoredValue::new(api_base);

    box_status_view(HostCell::new(BoxStatusHost {
        hostname,
        cpu_label,
        disk_label,
        os_label,
        updates_label,
        container_count,
        backup_label,
        backup_count,
        confirm_open,
        backup_busy,
        backup_msg,
        token,
        on_refresh,
        api_for_backup,
    }))
}

struct BoxStatusHost {
    hostname: String,
    cpu_label: String,
    disk_label: String,
    os_label: String,
    updates_label: String,
    container_count: String,
    backup_label: String,
    backup_count: String,
    confirm_open: RwSignal<bool>,
    backup_busy: RwSignal<bool>,
    backup_msg: RwSignal<String>,
    token: RwSignal<String>,
    on_refresh: Callback<()>,
    api_for_backup: StoredValue<String>,
}

impl Host for BoxStatusHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "hostname" => Some(Value::Str(self.hostname.clone())),
            "cpuLabel" => Some(Value::Str(self.cpu_label.clone())),
            "diskLabel" => Some(Value::Str(self.disk_label.clone())),
            "osLabel" => Some(Value::Str(self.os_label.clone())),
            "updatesLabel" => Some(Value::Str(self.updates_label.clone())),
            "containerCount" => Some(Value::Str(self.container_count.clone())),
            "backupLabel" => Some(Value::Str(self.backup_label.clone())),
            "backupCount" => Some(Value::Str(self.backup_count.clone())),
            "confirmOpen" => Some(Value::Bool(self.confirm_open.get())),
            "backupBusy" => Some(Value::Bool(self.backup_busy.get())),
            "backupMsg" => Some(Value::Str(self.backup_msg.get())),
            "hasBackupMsg" => Some(Value::Bool(!self.backup_msg.get().is_empty())),
            _ => None,
        }
    }

    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, HostError> {
        match name {
            "openConfirm" => {
                if !self.backup_busy.get_untracked() {
                    self.backup_msg.set(String::new());
                    self.confirm_open.set(true);
                }
            }
            "cancelConfirm" => self.confirm_open.set(false),
            "runBackup" => {
                self.confirm_open.set(false);
                self.backup_msg
                    .set("Running timestamped /etc backup…".into());
                let tok = {
                    let t = self.token.get();
                    if t.trim().is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                };
                let base = self.api_for_backup.get_value();
                let on_refresh = self.on_refresh;
                let backup_msg = self.backup_msg;
                spawn_busy(self.backup_busy, async move {
                    match post_backup_etc(&base, tok.as_deref()).await {
                        Ok(report) => {
                            backup_msg.set(format!(
                                "Backup complete: {} ({} entries).",
                                report.dest,
                                report.copied.len()
                            ));
                            on_refresh.run(());
                        }
                        Err(e) => backup_msg.set(e),
                    }
                });
            }
            _ => {}
        }
        Ok(Value::Unit)
    }
}

fn format_cpu(host: &HostMetrics) -> String {
    match (host.cpu_percent, host.load_1, host.load_5, host.load_15) {
        (Some(p), Some(a), Some(b), Some(c)) => {
            format!("{:.0}% · {a:.2} / {b:.2} / {c:.2}", p.round())
        }
        (Some(p), _, _, _) => format!("{:.0}%", p.round()),
        _ => "unknown".into(),
    }
}

fn format_disk(host: &HostMetrics) -> String {
    match (host.disk_used_bytes, host.disk_total_bytes) {
        (Some(used), Some(total)) => {
            format!("{} used / {} total", human_bytes(used), human_bytes(total))
        }
        _ => "unknown".into(),
    }
}

fn format_os(host: &HostMetrics) -> String {
    host.armbian_version
        .as_deref()
        .filter(|s| !s.is_empty())
        .map_or_else(
            || {
                let pretty = host
                    .os_pretty_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown");
                host.kernel
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| pretty.to_owned(), |k| format!("{pretty} · {k}"))
            },
            |ver| {
                host.armbian_board
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map_or_else(
                        || format!("Armbian {ver}"),
                        |board| format!("Armbian {ver} · {board}"),
                    )
            },
        )
}

fn format_updates(host: &HostMetrics) -> String {
    match host.apt_upgradable {
        Some(0) => "up to date".into(),
        Some(n) => format!("{n} pending"),
        None => "unknown".into(),
    }
}

fn human_bytes(n: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * K;
    const G: u64 = M * K;
    if n >= G {
        format!("{}G", n / G)
    } else if n >= M {
        format!("{}M", n / M)
    } else if n >= K {
        format!("{}K", n / K)
    } else {
        format!("{n}B")
    }
}
