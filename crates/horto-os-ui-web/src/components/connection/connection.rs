use js_sys::{Function, Object, Promise, Reflect};
use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::status::{connection_error_detail, connection_label, Snapshot};

include!(concat!(env!("OUT_DIR"), "/rangular/connection_view.rs"));

#[component]
pub fn ConnectionPanel(
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let ssh_host = RwSignal::new(String::new());
    let install_ssh_key = RwSignal::new(false);
    // Safe default: preview only. Uncheck + confirm for real remote apply.
    let remote_dry_run = RwSignal::new(true);
    let remote_busy = RwSignal::new(false);
    let remote_log = RwSignal::new(String::new());

    connection_view(HostCell::new(ConnectionHost {
        url,
        token,
        busy,
        snap,
        on_refresh,
        ssh_host,
        install_ssh_key,
        remote_dry_run,
        remote_busy,
        remote_log,
    }))
}

struct ConnectionHost {
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
    ssh_host: RwSignal<String>,
    install_ssh_key: RwSignal<bool>,
    remote_dry_run: RwSignal<bool>,
    remote_busy: RwSignal<bool>,
    remote_log: RwSignal<String>,
}

impl Host for ConnectionHost {
    fn get(&self, name: &str) -> Option<Value> {
        let snap = self.snap.get();
        let (label, class) = connection_label(&snap);
        let detail = connection_error_detail(&snap).unwrap_or_default();
        let remote_log = self.remote_log.get();
        let dry_run = self.remote_dry_run.get();
        match name {
            "url" => Some(Value::Str(self.url.get())),
            "token" => Some(Value::Str(self.token.get())),
            "busy" => Some(Value::Bool(self.busy.get())),
            "statusLabel" => Some(Value::Str(label)),
            "statusOk" => Some(Value::Bool(class == "status-ok")),
            "statusBad" => Some(Value::Bool(class == "status-bad")),
            "statusWarn" => Some(Value::Bool(class == "status-warn")),
            "hasError" => Some(Value::Bool(!detail.is_empty())),
            "errorDetail" => Some(Value::Str(detail)),
            "sshHost" => Some(Value::Str(self.ssh_host.get())),
            "installSshKey" => Some(Value::Bool(self.install_ssh_key.get())),
            "remoteDryRun" => Some(Value::Bool(dry_run)),
            "remoteSetupLabel" => Some(Value::Str(if dry_run {
                "Remote dry-run setup".into()
            } else {
                "Remote apply setup".into()
            })),
            "remoteBusy" => Some(Value::Bool(self.remote_busy.get())),
            "remoteLog" => Some(Value::Str(remote_log.clone())),
            "hasRemoteLog" => Some(Value::Bool(!remote_log.is_empty())),
            _ => None,
        }
    }

    fn set(&mut self, name: &str, value: Value) -> Result<(), HostError> {
        if let Some(s) = value.as_str() {
            match name {
                "url" => self.url.set(s.to_owned()),
                "token" => {
                    self.token.set(s.to_owned());
                    crate::save_api_token(s);
                }
                "sshHost" => self.ssh_host.set(s.to_owned()),
                _ => {}
            }
        }
        if let Some(b) = value.as_bool() {
            match name {
                "installSshKey" => self.install_ssh_key.set(b),
                "remoteDryRun" => self.remote_dry_run.set(b),
                _ => {}
            }
        }
        Ok(())
    }

    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, HostError> {
        if name == "refresh" && !self.busy.get() {
            self.on_refresh.run(());
        }
        if name == "remoteSetup" && !self.remote_busy.get() {
            let host = self.ssh_host.get().trim().to_owned();
            if host.is_empty() {
                self.remote_log
                    .set("Set an OpenSSH Host alias or user@host first.".into());
                return Ok(Value::Unit);
            }
            let install_ssh_key = self.install_ssh_key.get();
            let dry_run = self.remote_dry_run.get();
            if !dry_run {
                let Some(window) = web_sys::window() else {
                    self.remote_log
                        .set("No window; cannot confirm apply.".into());
                    return Ok(Value::Unit);
                };
                let ok = window
                    .confirm_with_message(
                        "Apply remote setup on the box? This installs CLI, TUI, and status-api over SSH (sudo).",
                    )
                    .unwrap_or(false);
                if !ok {
                    self.remote_log.set("Remote apply cancelled.".into());
                    return Ok(Value::Unit);
                }
            }
            self.remote_busy.set(true);
            self.remote_log.set(if dry_run {
                "Starting remote dry-run setup…".into()
            } else {
                "Starting remote apply setup…".into()
            });
            let remote_busy = self.remote_busy;
            let remote_log = self.remote_log;
            let token = self.token;
            leptos::task::spawn_local(async move {
                match invoke_remote_setup(&host, install_ssh_key, dry_run).await {
                    Ok(result) => {
                        let mut log = result.log;
                        if let Some(api_token) = result.api_token {
                            let save = web_sys::window()
                                .and_then(|w| {
                                    w.confirm_with_message(
                                        "Save the status-api bearer into Connection (localStorage)?",
                                    )
                                    .ok()
                                })
                                .unwrap_or(false);
                            if save {
                                token.set(api_token.clone());
                                crate::save_api_token(&api_token);
                                if !log.is_empty() {
                                    log.push('\n');
                                }
                                log.push_str("Saved status-api bearer into Connection.");
                            }
                        }
                        remote_log.set(log);
                    }
                    Err(e) => remote_log.set(e),
                }
                remote_busy.set(false);
            });
        }
        Ok(Value::Unit)
    }
}

struct RemoteSetupUiResult {
    log: String,
    api_token: Option<String>,
}

async fn invoke_remote_setup(
    host: &str,
    install_ssh_key: bool,
    dry_run: bool,
) -> Result<RemoteSetupUiResult, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
    let tauri = Reflect::get(&window, &"__TAURI__".into()).map_err(|_| {
        "Remote install needs the Horto desktop app (Tauri). Browser-only builds cannot SSH."
            .to_owned()
    })?;
    if tauri.is_undefined() || tauri.is_null() {
        return Err(
            "Remote install needs the Horto desktop app (Tauri). Browser-only builds cannot SSH."
                .into(),
        );
    }
    let core = Reflect::get(&tauri, &"core".into()).map_err(|e| format!("{e:?}"))?;
    let invoke = Reflect::get(&core, &"invoke".into()).map_err(|e| format!("{e:?}"))?;
    let invoke: Function = invoke
        .dyn_into()
        .map_err(|_| "invoke is not a function".to_owned())?;

    let args = Object::new();
    let payload = Object::new();
    Reflect::set(&payload, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"installSshKey".into(), &install_ssh_key.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"dryRun".into(), &dry_run.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"full".into(), &true.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&args, &"args".into(), &payload).map_err(|e| format!("{e:?}"))?;

    let promise = invoke
        .call2(&core, &"remote_setup".into(), &args)
        .map_err(|e| format!("invoke failed: {e:?}"))?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "invoke did not return a Promise".to_owned())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("remote_setup error: {e:?}"))?;

    if let Ok(log_v) = Reflect::get(&value, &"log".into()) {
        let log = log_v.as_string().unwrap_or_else(|| format!("{log_v:?}"));
        let api_token = Reflect::get(&value, &"apiToken".into())
            .ok()
            .and_then(|v| v.as_string())
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        return Ok(RemoteSetupUiResult { log, api_token });
    }

    Ok(RemoteSetupUiResult {
        log: value.as_string().unwrap_or_else(|| format!("{value:?}")),
        api_token: None,
    })
}
