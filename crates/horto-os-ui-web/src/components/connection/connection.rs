use js_sys::{Object, Reflect};
use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::busy::{spawn_busy, spawn_busy_force};
use crate::status::{connection_error_detail, connection_label, Snapshot};
use crate::tauri_bridge::{
    invoke_list_known_remote_hosts, invoke_list_release_tags, invoke_remote_setup_cmd,
    invoke_remote_surfaces_probe, is_desktop_shell, KnownHostUi,
};

include!(concat!(env!("OUT_DIR"), "/rangular/connection_view.rs"));

const HOSTS_RELOAD_TIMEOUT_MS: u32 = 8_000;
const HOSTS_RELOAD_MIN_BUSY_MS: f64 = 300.0;
const HOSTS_HINT_PICK: &str = "Pick where Status API and probes go. localhost is this PC.";
const HOSTS_HINT_EMPTY: &str = "No hosts found. Type a name below.";

#[component]
pub fn ConnectionPanel(
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let ssh_host = RwSignal::new(String::new());
    let known_hosts = RwSignal::new(Vec::<KnownHostUi>::new());
    let hosts_busy = RwSignal::new(false);
    let hosts_hint = RwSignal::new(String::from(HOSTS_HINT_PICK));
    let install_ssh_key = RwSignal::new(false);
    // Safe default: apply off. Check Apply and confirm for real remote changes.
    let remote_apply = RwSignal::new(false);
    let release_tag = RwSignal::new(String::from("dev-preview"));
    let release_tags = RwSignal::new(vec![String::from("dev-preview")]);
    let install_status_api = RwSignal::new(true);
    let install_mcp = RwSignal::new(true);
    let stack_dockge = RwSignal::new(false);
    let stack_open_webui = RwSignal::new(false);
    let stack_evcc = RwSignal::new(false);
    let stack_whisper = RwSignal::new(false);
    let stack_deepseek = RwSignal::new(false);
    let stack_piper = RwSignal::new(false);
    let stack_openwakeword = RwSignal::new(false);
    let remote_busy = RwSignal::new(false);
    let remote_log = RwSignal::new(String::new());
    let surfaces_busy = RwSignal::new(false);
    let surfaces_text = RwSignal::new(String::new());
    let surface_ssh = RwSignal::new(String::from("?"));
    let surface_cli = RwSignal::new(String::from("?"));
    let surface_api = RwSignal::new(String::from("?"));
    let surface_mcp_pc = RwSignal::new(String::from("?"));
    let surface_mcp_box = RwSignal::new(String::from("?"));

    // Boot once. Creating HostCell inside a reactive view! remounts handlers.
    let booted = StoredValue::new(false);
    Effect::new(move |_| {
        if booted.get_value() {
            return;
        }
        booted.set_value(true);
        reload_known_hosts(known_hosts, hosts_hint, hosts_busy);
        reload_release_tags(release_tags, release_tag);
    });

    connection_view(HostCell::new(ConnectionHost {
        url,
        token,
        busy,
        snap,
        on_refresh,
        ssh_host,
        known_hosts,
        hosts_busy,
        hosts_hint,
        install_ssh_key,
        remote_apply,
        release_tag,
        release_tags,
        install_status_api,
        install_mcp,
        stack_dockge,
        stack_open_webui,
        stack_evcc,
        stack_whisper,
        stack_deepseek,
        stack_piper,
        stack_openwakeword,
        remote_busy,
        remote_log,
        surfaces_busy,
        surfaces_text,
        surface_ssh,
        surface_cli,
        surface_api,
        surface_mcp_pc,
        surface_mcp_box,
    }))
}

struct ConnectionHost {
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
    ssh_host: RwSignal<String>,
    known_hosts: RwSignal<Vec<KnownHostUi>>,
    hosts_busy: RwSignal<bool>,
    hosts_hint: RwSignal<String>,
    install_ssh_key: RwSignal<bool>,
    remote_apply: RwSignal<bool>,
    release_tag: RwSignal<String>,
    release_tags: RwSignal<Vec<String>>,
    install_status_api: RwSignal<bool>,
    install_mcp: RwSignal<bool>,
    stack_dockge: RwSignal<bool>,
    stack_open_webui: RwSignal<bool>,
    stack_evcc: RwSignal<bool>,
    stack_whisper: RwSignal<bool>,
    stack_deepseek: RwSignal<bool>,
    stack_piper: RwSignal<bool>,
    stack_openwakeword: RwSignal<bool>,
    remote_busy: RwSignal<bool>,
    remote_log: RwSignal<String>,
    surfaces_busy: RwSignal<bool>,
    surfaces_text: RwSignal<String>,
    surface_ssh: RwSignal<String>,
    surface_cli: RwSignal<String>,
    surface_api: RwSignal<String>,
    surface_mcp_pc: RwSignal<String>,
    surface_mcp_box: RwSignal<String>,
}

impl Host for ConnectionHost {
    fn get(&self, name: &str) -> Option<Value> {
        let snap = self.snap.get();
        let (label, class) = connection_label(&snap);
        let detail = connection_error_detail(&snap).unwrap_or_default();
        let remote_log = self.remote_log.get();
        let apply = self.remote_apply.get();
        let hosts = self.known_hosts.get();
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
            "knownHosts" => Some(Value::List(
                hosts.iter().map(|h| Value::Str(h.name.clone())).collect(),
            )),
            "hasKnownHosts" => Some(Value::Bool(!hosts.is_empty())),
            "hostsBusy" => Some(Value::Bool(self.hosts_busy.get())),
            "hostsBusyLabel" => Some(Value::Str("Reloading hosts…".into())),
            "hostsHint" => Some(Value::Str(self.hosts_hint.get())),
            "installSshKey" => Some(Value::Bool(self.install_ssh_key.get())),
            "remoteApply" => Some(Value::Bool(apply)),
            "releaseTag" => Some(Value::Str(self.release_tag.get())),
            "releaseTags" => Some(Value::List(
                self.release_tags
                    .get()
                    .into_iter()
                    .map(Value::Str)
                    .collect(),
            )),
            "releasesHeading" => {
                let n = self.release_tags.get().len();
                Some(Value::Str(format!("Releases ({n})")))
            }
            "installStatusApi" => Some(Value::Bool(self.install_status_api.get())),
            "installMcp" => Some(Value::Bool(self.install_mcp.get())),
            "stackDockge" => Some(Value::Bool(self.stack_dockge.get())),
            "stackOpenWebui" => Some(Value::Bool(self.stack_open_webui.get())),
            "stackEvcc" => Some(Value::Bool(self.stack_evcc.get())),
            "stackWhisper" => Some(Value::Bool(self.stack_whisper.get())),
            "stackDeepseek" => Some(Value::Bool(self.stack_deepseek.get())),
            "stackPiper" => Some(Value::Bool(self.stack_piper.get())),
            "stackOpenwakeword" => Some(Value::Bool(self.stack_openwakeword.get())),
            "remoteSetupLabel" => Some(Value::Str(if apply {
                "Install on box".into()
            } else {
                "Preview install".into()
            })),
            "remoteBusy" => Some(Value::Bool(self.remote_busy.get())),
            "remoteBusyLabel" => Some(Value::Str(if self.remote_apply.get() {
                format!("Installing on {}…", self.ssh_host.get())
            } else {
                format!("Previewing install for {}…", self.ssh_host.get())
            })),
            "remoteLog" => Some(Value::Str(remote_log)),
            "hasRemoteLog" => Some(Value::Bool(!remote_log.is_empty())),
            "surfacesBusy" => Some(Value::Bool(self.surfaces_busy.get())),
            "probeBusyLabel" => {
                let host = self.ssh_host.get();
                let host = host.trim();
                if host.is_empty() {
                    Some(Value::Str("Probing…".into()))
                } else {
                    Some(Value::Str(format!("Probing {host}…")))
                }
            }
            "surfacesText" => Some(Value::Str(self.surfaces_text.get())),
            "hasSurfacesText" => Some(Value::Bool(!self.surfaces_text.get().is_empty())),
            "hasSurfaces" => Some(Value::Bool(self.surface_ssh.get() != "?")),
            "surfaceSsh" => Some(Value::Str(self.surface_ssh.get())),
            "surfaceCli" => Some(Value::Str(self.surface_cli.get())),
            "surfaceApi" => Some(Value::Str(self.surface_api.get())),
            "surfaceMcpPc" => Some(Value::Str(self.surface_mcp_pc.get())),
            "surfaceMcpBox" => Some(Value::Str(self.surface_mcp_box.get())),
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
                "sshHost" => {
                    self.ssh_host.set(s.to_owned());
                    clear_stale_host_prompts(self.surfaces_text, self.remote_log);
                }
                "releaseTag" => self.release_tag.set(s.to_owned()),
                _ => {}
            }
        }
        if let Some(b) = value.as_bool() {
            match name {
                "installSshKey" => self.install_ssh_key.set(b),
                "remoteApply" => self.remote_apply.set(b),
                "installStatusApi" => self.install_status_api.set(b),
                "installMcp" => self.install_mcp.set(b),
                "stackDockge" => self.stack_dockge.set(b),
                "stackOpenWebui" => self.stack_open_webui.set(b),
                "stackEvcc" => self.stack_evcc.set(b),
                "stackWhisper" => self.stack_whisper.set(b),
                "stackDeepseek" => self.stack_deepseek.set(b),
                "stackPiper" => self.stack_piper.set(b),
                "stackOpenwakeword" => self.stack_openwakeword.set(b),
                _ => {}
            }
        }
        Ok(())
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, HostError> {
        if name == "refresh" && !self.busy.get_untracked() {
            self.on_refresh.run(());
        }
        if name == "reloadHosts" {
            reload_known_hosts(self.known_hosts, self.hosts_hint, self.hosts_busy);
        }
        if name == "pickHost" {
            if let Some(host) = args.first().and_then(Value::as_str) {
                apply_picked_host(
                    host,
                    self.ssh_host,
                    self.url,
                    self.surfaces_text,
                    self.remote_log,
                );
                if !self.busy.get_untracked() {
                    self.on_refresh.run(());
                }
            }
        }
        if name == "pickRelease" {
            if let Some(tag) = args.first().and_then(Value::as_str) {
                let tag = tag.trim();
                if !tag.is_empty() {
                    self.release_tag.set(tag.to_owned());
                }
            }
        }
        if name == "probeSurfaces" {
            let host = self.ssh_host.get().trim().to_owned();
            if host.is_empty() {
                self.surfaces_text.set("Pick or enter a host first.".into());
                return Ok(Value::Unit);
            }
            self.surfaces_text.set(String::new());
            let surfaces_text = self.surfaces_text;
            let surface_ssh = self.surface_ssh;
            let surface_cli = self.surface_cli;
            let surface_api = self.surface_api;
            let surface_mcp_pc = self.surface_mcp_pc;
            let surface_mcp_box = self.surface_mcp_box;
            spawn_busy(self.surfaces_busy, async move {
                match invoke_remote_surfaces(&host).await {
                    Ok(report) => {
                        surface_ssh.set(report.ssh.clone());
                        surface_cli.set(report.cli.clone());
                        surface_api.set(report.api.clone());
                        surface_mcp_pc.set(report.mcp_pc.clone());
                        surface_mcp_box.set(report.mcp_box.clone());
                        surfaces_text.set(report.text);
                    }
                    Err(e) => {
                        surface_ssh.set("?".into());
                        surface_cli.set("?".into());
                        surface_api.set("?".into());
                        surface_mcp_pc.set("?".into());
                        surface_mcp_box.set("?".into());
                        surfaces_text.set(e);
                    }
                }
            });
        }
        if name == "remoteSetup" {
            let host = self.ssh_host.get().trim().to_owned();
            if host.is_empty() {
                self.remote_log.set("Pick or enter a host first.".into());
                return Ok(Value::Unit);
            }
            let install_ssh_key = self.install_ssh_key.get();
            let apply = self.remote_apply.get();
            let release_tag = self.release_tag.get().trim().to_owned();
            let release_tag = if release_tag.is_empty() {
                "dev-preview".to_owned()
            } else {
                release_tag
            };
            let install_status_api = self.install_status_api.get();
            let install_mcp = self.install_mcp.get();
            let stacks = stacks_csv(
                self.stack_dockge.get(),
                self.stack_open_webui.get(),
                self.stack_evcc.get(),
                self.stack_whisper.get(),
                self.stack_deepseek.get(),
                self.stack_piper.get(),
                self.stack_openwakeword.get(),
            );
            if apply {
                let Some(window) = web_sys::window() else {
                    self.remote_log
                        .set("No window; cannot confirm apply.".into());
                    return Ok(Value::Unit);
                };
                let ok = window
                    .confirm_with_message(
                        "Apply remote setup on the box over SSH (sudo)? Status-api / MCP / apps follow the checkboxes.",
                    )
                    .unwrap_or(false);
                if !ok {
                    self.remote_log.set("Remote apply cancelled.".into());
                    return Ok(Value::Unit);
                }
            }
            self.remote_log.set(if apply {
                "Starting install…".into()
            } else {
                "Starting install preview…".into()
            });
            let remote_log = self.remote_log;
            let token = self.token;
            spawn_busy(self.remote_busy, async move {
                match invoke_remote_setup(
                    &host,
                    install_ssh_key,
                    apply,
                    &release_tag,
                    install_status_api,
                    install_mcp,
                    &stacks,
                )
                .await
                {
                    Ok(result) => {
                        let mut log = result.log;
                        if let Some(api_token) = result.api_token {
                            let save = web_sys::window()
                                .and_then(|w| {
                                    w.confirm_with_message(
                                        "Save the status-api bearer into Connection?",
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
            });
        }
        Ok(Value::Unit)
    }
}

fn apply_picked_host(
    host: &str,
    ssh_host: RwSignal<String>,
    url: RwSignal<String>,
    surfaces_text: RwSignal<String>,
    remote_log: RwSignal<String>,
) {
    let host = host.trim();
    if host.is_empty() {
        return;
    }
    ssh_host.set(host.to_owned());
    let next = status_api_url_for_host(&url.get_untracked(), host);
    url.set(next.clone());
    crate::save_status_api_url(&next);
    clear_stale_host_prompts(surfaces_text, remote_log);
}

/// Retarget the Status API URL hostname while keeping scheme, port, and path.
fn status_api_url_for_host(current_url: &str, host: &str) -> String {
    let host = host.trim();
    if host.is_empty() {
        return current_url.to_owned();
    }
    if let Ok(parsed) = web_sys::Url::new(current_url.trim()) {
        parsed.set_hostname(host);
        return parsed.href();
    }
    format!("http://{host}:8787")
}

fn clear_stale_host_prompts(surfaces_text: RwSignal<String>, remote_log: RwSignal<String>) {
    let clear_if_prompt = |sig: RwSignal<String>| {
        let cur = sig.get();
        if cur.contains("Pick or enter a host") || cur.contains("Set an OpenSSH") {
            sig.set(String::new());
        }
    };
    clear_if_prompt(surfaces_text);
    clear_if_prompt(remote_log);
}

fn reload_known_hosts(
    known_hosts: RwSignal<Vec<KnownHostUi>>,
    hosts_hint: RwSignal<String>,
    hosts_busy: RwSignal<bool>,
) {
    if !is_desktop_shell() {
        known_hosts.set(Vec::new());
        set_hosts_hint(hosts_hint, "Host list needs the Desktop app.");
        return;
    }
    // Keep the pick hint stable while busy (busy state is on the button only).
    spawn_busy_force(hosts_busy, async move {
        let started = js_sys::Date::now();
        let finished = std::rc::Rc::new(std::cell::Cell::new(false));
        let finished_watch = finished.clone();
        let hint_watch = hosts_hint;
        let busy_watch = hosts_busy;
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(HOSTS_RELOAD_TIMEOUT_MS).await;
            if !finished_watch.get() {
                set_hosts_hint(hint_watch, "Reload hosts timed out. Try again.");
                busy_watch.set(false);
            }
        });
        match invoke_list_known_remote_hosts().await {
            Ok(list) => {
                set_hosts_hint(hosts_hint, &hosts_picker_hint(list.len()));
                known_hosts.set(list);
            }
            Err(e) => {
                known_hosts.set(Vec::new());
                set_hosts_hint(hosts_hint, &format!("Could not load hosts: {e}"));
            }
        }
        finished.set(true);
        let elapsed = js_sys::Date::now() - started;
        if elapsed < HOSTS_RELOAD_MIN_BUSY_MS {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let wait_ms = (HOSTS_RELOAD_MIN_BUSY_MS - elapsed) as u32;
            gloo_timers::future::TimeoutFuture::new(wait_ms).await;
        }
    });
}

fn set_hosts_hint(hosts_hint: RwSignal<String>, next: &str) {
    if hosts_hint.get_untracked() != next {
        hosts_hint.set(next.to_owned());
    }
}

fn hosts_picker_hint(count: usize) -> String {
    if count == 0 {
        return HOSTS_HINT_EMPTY.into();
    }
    HOSTS_HINT_PICK.into()
}

fn reload_release_tags(release_tags: RwSignal<Vec<String>>, release_tag: RwSignal<String>) {
    if !is_desktop_shell() {
        return;
    }
    leptos::task::spawn_local(async move {
        match invoke_list_release_tags().await {
            Ok(tags) if !tags.is_empty() => {
                let current = release_tag.get_untracked();
                if !tags.iter().any(|t| t == &current) {
                    release_tag.set(tags[0].clone());
                }
                release_tags.set(tags);
            }
            Ok(_) => {}
            Err(_e) => {
                // Keep the tip fallback already in the signal.
            }
        }
    });
}

struct SurfacesUiReport {
    text: String,
    ssh: String,
    cli: String,
    api: String,
    mcp_pc: String,
    mcp_box: String,
}

async fn invoke_remote_surfaces(host: &str) -> Result<SurfacesUiReport, String> {
    let value = invoke_remote_surfaces_probe(host).await?;

    let local = Reflect::get(&value, &"local_version".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let ssh_obj = Reflect::get(&value, &"ssh".into()).map_err(|e| format!("{e:?}"))?;
    let cli_obj = Reflect::get(&value, &"cli".into()).map_err(|e| format!("{e:?}"))?;
    let api_obj = Reflect::get(&value, &"api".into()).map_err(|e| format!("{e:?}"))?;
    let mcp_pc_obj = Reflect::get(&value, &"mcp_pc".into()).map_err(|e| format!("{e:?}"))?;
    let mcp_box_obj = Reflect::get(&value, &"mcp_box".into()).map_err(|e| format!("{e:?}"))?;

    let ssh_status = Reflect::get(&ssh_obj, &"status".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "?".into());
    let cli_status = cli_status_label(&cli_obj);
    let api_health = Reflect::get(&api_obj, &"health".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "?".into());
    let api_url = Reflect::get(&api_obj, &"url".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let mcp_pc_image = Reflect::get(&mcp_pc_obj, &"docker".into())
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_string().filter(|s| !s.is_empty())
            }
        });
    let mcp_pc_bin = Reflect::get(&mcp_pc_obj, &"binary".into())
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_string().filter(|s| !s.is_empty())
            }
        });
    let mcp_pc_reach = Reflect::get(&mcp_pc_obj, &"http_reach".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "?".into());
    let mcp_box_reach = Reflect::get(&mcp_box_obj, &"http_reach".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "?".into());
    let mcp_box_image = Reflect::get(&mcp_box_obj, &"docker".into())
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_string().filter(|s| !s.is_empty())
            }
        });
    let mcp_box_bin = Reflect::get(&mcp_box_obj, &"binary".into())
        .ok()
        .and_then(|v| {
            if v.is_null() || v.is_undefined() {
                None
            } else {
                v.as_string().filter(|s| !s.is_empty())
            }
        });

    let mcp_pc_summary = {
        let image = if mcp_pc_image.is_some() { "yes" } else { "no" };
        let binary = if mcp_pc_bin.is_some() { "yes" } else { "no" };
        let stdio = if mcp_pc_image.is_some() || mcp_pc_bin.is_some() {
            "ready"
        } else {
            "no"
        };
        if mcp_pc_reach == "ok" {
            format!("image:{image} · binary:{binary} · stdio:{stdio} · http:ok")
        } else {
            format!("image:{image} · binary:{binary} · stdio:{stdio}")
        }
    };
    let mcp_box_summary = {
        let image = if mcp_box_image.is_some() { "yes" } else { "no" };
        let binary = if mcp_box_bin.is_some() { "yes" } else { "no" };
        let stdio = if mcp_box_image.is_some() || mcp_box_bin.is_some() {
            "ready"
        } else {
            "no"
        };
        if mcp_box_reach == "ok" {
            format!("image:{image} · binary:{binary} · stdio:{stdio} · http:ok")
        } else {
            format!("image:{image} · binary:{binary} · stdio:{stdio}")
        }
    };

    let text = format!(
        "local={local}\nssh={ssh_status}\ncli={cli_status}\napi={api_url} health={api_health}\nmcp_pc image={} binary={} http={mcp_pc_reach}\nmcp_box image={} binary={} http={mcp_box_reach}\n",
        mcp_pc_image.as_deref().unwrap_or("missing"),
        mcp_pc_bin.as_deref().unwrap_or("missing"),
        mcp_box_image.as_deref().unwrap_or("missing"),
        mcp_box_bin.as_deref().unwrap_or("missing"),
    );

    Ok(SurfacesUiReport {
        text,
        ssh: ssh_status,
        cli: cli_status,
        api: format!("{api_health} ({api_url})"),
        mcp_pc: mcp_pc_summary,
        mcp_box: mcp_box_summary,
    })
}

fn cli_status_label(cli_obj: &wasm_bindgen::JsValue) -> String {
    let status = Reflect::get(cli_obj, &"status".into()).ok();
    let Some(status) = status else {
        return "?".into();
    };
    if let Some(s) = status.as_string() {
        return s;
    }
    // RemoteBoxCliStatus::Found serializes as { "found": "version" } (snake_case).
    if let Ok(found) = Reflect::get(&status, &"found".into()) {
        if let Some(v) = found.as_string() {
            return v;
        }
    }
    format!("{status:?}")
}

struct RemoteSetupUiResult {
    log: String,
    api_token: Option<String>,
}

async fn invoke_remote_setup(
    host: &str,
    install_ssh_key: bool,
    apply: bool,
    release_tag: &str,
    install_status_api: bool,
    install_mcp: bool,
    stacks: &str,
) -> Result<RemoteSetupUiResult, String> {
    let payload = Object::new();
    Reflect::set(&payload, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"installSshKey".into(), &install_ssh_key.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"apply".into(), &apply.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"releaseTag".into(), &release_tag.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"installStatusApi".into(),
        &install_status_api.into(),
    )
    .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"installMcp".into(), &install_mcp.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"stacks".into(), &stacks.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"full".into(), &true.into()).map_err(|e| format!("{e:?}"))?;

    let value = invoke_remote_setup_cmd(&payload).await?;

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

fn stacks_csv(
    dockge: bool,
    open_webui: bool,
    evcc: bool,
    whisper: bool,
    deepseek: bool,
    piper: bool,
    openwakeword: bool,
) -> String {
    let mut parts = Vec::new();
    if dockge {
        parts.push("dockge");
    }
    if open_webui {
        parts.push("open-webui");
    }
    if evcc {
        parts.push("evcc");
    }
    if whisper {
        parts.push("whisper");
    }
    if deepseek {
        parts.push("deepseek");
    }
    if piper {
        parts.push("piper");
    }
    if openwakeword {
        parts.push("openwakeword");
    }
    parts.join(",")
}
