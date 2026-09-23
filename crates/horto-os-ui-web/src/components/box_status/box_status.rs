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

    let open_confirm = move |_| {
        if !backup_busy.get_untracked() {
            backup_msg.set(String::new());
            confirm_open.set(true);
        }
    };
    let cancel_confirm = move |_| confirm_open.set(false);
    let run_backup = move |_| {
        confirm_open.set(false);
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
        spawn_busy(backup_busy, async move {
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
    };

    view! {
        <section class="box-status panel" aria-label="Box">
            <h2>"Box"</h2>
            {box_status_view(HostCell::new(BoxStatusHost {
                hostname,
                container_count,
                backup_label,
                backup_count,
            }))}
            <footer class="box-status__footer">
                <div class="box-status__footer-text">
                    <span class="box-status__footer-label">"/etc backup"</span>
                    <p class="box-status__footer-hint">
                        "Timestamped copy on the box. Does not change running config."
                    </p>
                    <Show when=move || backup_busy.get() fallback=|| ()>
                        <p class="horto-busy-status is-visible" aria-live="polite">
                            "Backing up…"
                        </p>
                    </Show>
                    <Show when=move || !backup_msg.get().is_empty() fallback=|| ()>
                        <p class="box-status__msg">{move || backup_msg.get()}</p>
                    </Show>
                </div>
                <button
                    type="button"
                    class=move || {
                        if backup_busy.get() {
                            "horto-btn horto-btn--ghost horto-btn--busy"
                        } else {
                            "horto-btn horto-btn--ghost"
                        }
                    }
                    disabled=move || backup_busy.get()
                    on:click=open_confirm
                >
                    "Backup /etc"
                </button>
            </footer>
        </section>

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
                        "Requires the bearer token from Connection."
                    </p>
                    <div class="box-status__confirm-row">
                        <button
                            type="button"
                            class="horto-btn horto-btn--ghost"
                            on:click=cancel_confirm
                        >
                            "Cancel"
                        </button>
                        <button type="button" class="horto-btn" on:click=run_backup>
                            "Confirm backup"
                        </button>
                    </div>
                </div>
            </div>
        </Show>
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
