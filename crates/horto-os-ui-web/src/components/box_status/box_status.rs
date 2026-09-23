use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::busy::spawn_busy;
use crate::status::{post_backup_etc, BoxStatus};

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

    let confirm_open = RwSignal::new(false);
    let backup_busy = RwSignal::new(false);
    let backup_msg = RwSignal::new(String::new());
    let api_for_backup = StoredValue::new(api_base);

    box_status_view(HostCell::new(BoxStatusHost {
        hostname,
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
