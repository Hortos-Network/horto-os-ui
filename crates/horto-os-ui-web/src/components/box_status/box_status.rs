use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

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

    let open_confirm = move |_| {
        if !backup_busy.get() {
            backup_msg.set(String::new());
            confirm_open.set(true);
        }
    };
    let cancel_confirm = move |_| confirm_open.set(false);
    let run_backup = move |_| {
        if backup_busy.get() {
            return;
        }
        confirm_open.set(false);
        backup_busy.set(true);
        backup_msg.set("Running timestamped /etc backup…".into());
        let tok = {
            let t = token.get();
            if t.trim().is_empty() {
                None
            } else {
                Some(t)
            }
        };
        let base = api_for_backup.get_value();
        leptos::task::spawn_local(async move {
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
            backup_busy.set(false);
        });
    };

    view! {
        <div class="box-status-wrap">
            {box_status_view(HostCell::new(BoxStatusHost {
                hostname,
                container_count,
                backup_label,
                backup_count,
            }))}
            <div class="box-status__actions">
                <button
                    type="button"
                    class="box-status__backup"
                    disabled=move || backup_busy.get()
                    on:click=open_confirm
                >
                    {move || {
                        if backup_busy.get() {
                            "Backing up…"
                        } else {
                            "Backup /etc"
                        }
                    }}
                </button>
                <Show when=move || !backup_msg.get().is_empty() fallback=|| ()>
                    <p class="box-status__msg">{move || backup_msg.get()}</p>
                </Show>
            </div>

            <Show when=move || confirm_open.get() fallback=|| ()>
                <div class="about-backdrop" on:click=cancel_confirm>
                    <div
                        class="about-dialog"
                        role="dialog"
                        aria-labelledby="backup-confirm-title"
                        on:click=move |ev| ev.stop_propagation()
                    >
                        <h2 id="backup-confirm-title">"Confirm /etc backup"</h2>
                        <p>
                            "Creates a timestamped copy under /srv/backup/etc. \
                             Does not reinstall the box or change running config."
                        </p>
                        <p class="about-meta">
                            "Requires the bearer token from Connection (HORTO_API_TOKEN)."
                        </p>
                        <div class="box-status__confirm-row">
                            <button type="button" class="about-close" on:click=cancel_confirm>
                                "Cancel"
                            </button>
                            <button type="button" class="box-status__confirm" on:click=run_backup>
                                "Confirm backup"
                            </button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

struct BoxStatusHost {
    hostname: String,
    container_count: String,
    backup_label: String,
    backup_count: String,
}

impl Host for BoxStatusHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "hostname" => Some(Value::Str(self.hostname.clone())),
            "containerCount" => Some(Value::Str(self.container_count.clone())),
            "backupLabel" => Some(Value::Str(self.backup_label.clone())),
            "backupCount" => Some(Value::Str(self.backup_count.clone())),
            _ => None,
        }
    }

    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, HostError> {
        Ok(Value::Unit)
    }
}
