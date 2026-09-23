use js_sys::{Object, Reflect};
use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};
use wasm_bindgen::JsCast;

use crate::busy::{spawn_busy, spawn_busy_force};
use crate::components::{app_log_error, app_log_info};
use crate::status::{connection_error_detail, connection_label, Snapshot};
use crate::tauri_bridge::{
    invoke_list_known_remote_hosts, invoke_list_release_tags, invoke_remote_setup_cmd,
    invoke_remote_surfaces_probe, invoke_tip_cli_version, is_desktop_shell, KnownHostUi,
};

use super::state::ConnectionState;

include!(concat!(env!("OUT_DIR"), "/rangular/connection_view.rs"));

const HOSTS_RELOAD_TIMEOUT_MS: u32 = 8_000;
const HOSTS_RELOAD_MIN_BUSY_MS: f64 = 300.0;
const HOSTS_HINT_PICK: &str = "Pick where Status API and probes go. localhost is this PC.";
const HOSTS_HINT_EMPTY: &str = "No hosts found. Type a name below.";

/// One-shot Desktop boot for Connection (hosts, release tags, tip CLI).
///
/// Call once from App. Safe to call again: no-ops after the first run.
pub fn boot_connection(state: ConnectionState) {
    if state.is_booted() {
        return;
    }
    state.mark_booted();
    reload_known_hosts(state.known_hosts, state.hosts_hint, state.hosts_busy);
    reload_release_tags(state.release_tags, state.release_tag);
    reload_tip_cli_version(state.tip_cli_version);
    attach_install_log_listener(state);
}

#[component]
pub fn ConnectionPanel(
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
    state: ConnectionState,
) -> impl IntoView {
    // When Status API reports cli_version, sync box version without SSH.
    Effect::new(move |_| {
        let Some(api_ver) = snap.get().api_cli_version.filter(|s| !s.trim().is_empty()) else {
            return;
        };
        let tip = state.tip_cli_version.get();
        let current = cli_versions_match(&tip, &api_ver);
        state.box_cli_version.set(api_ver.clone());
        state.cli_probed.set(true);
        state.cli_current.set(current);
        if current {
            state.allow_stale_cli.set(false);
        }
        state
            .surface_cli
            .set(format_cli_surface_label(&api_ver, &tip, current));
    });

    // HostCell once per mount. Durable fields live in `state` (App-owned).
    connection_view(HostCell::new(ConnectionHost {
        url,
        token,
        busy,
        snap,
        on_refresh,
        state,
    }))
}

struct ConnectionHost {
    url: RwSignal<String>,
    token: RwSignal<String>,
    busy: RwSignal<bool>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
    state: ConnectionState,
}

impl ConnectionHost {
    fn install_options_all_on(&self) -> bool {
        self.state.install_ssh_key.get()
            && self.state.install_status_api.get()
            && self.state.install_mcp.get()
    }

    fn extra_apps_all_on(&self) -> bool {
        self.state.stack_dockge.get()
            && self.state.stack_open_webui.get()
            && self.state.stack_evcc.get()
            && self.state.stack_whisper.get()
            && self.state.stack_deepseek.get()
            && self.state.stack_piper.get()
            && self.state.stack_openwakeword.get()
    }

    fn remote_setup_preflight(&self) -> Result<(), String> {
        let host = self.state.ssh_host.get().trim().to_owned();
        if host.is_empty() {
            return Err("Pick or enter a host first.".into());
        }
        let probed = self.state.cli_probed.get();
        let current = self.state.cli_current.get();
        let allow_stale = self.state.allow_stale_cli.get();
        if !probed {
            return Err("Probe or Update CLI before install.".into());
        }
        if !current && !allow_stale {
            return Err("Box CLI is behind. Update CLI, or allow older CLI.".into());
        }
        Ok(())
    }

    fn begin_remote_setup(&self) {
        let host = self.state.ssh_host.get().trim().to_owned();
        let install_ssh_key = self.state.install_ssh_key.get();
        let apply = self.state.remote_apply.get();
        let allow_stale = self.state.allow_stale_cli.get();
        let release_tag = self.state.release_tag.get().trim().to_owned();
        let release_tag = if release_tag.is_empty() {
            "dev-preview".to_owned()
        } else {
            release_tag
        };
        let install_status_api = self.state.install_status_api.get();
        let install_mcp = self.state.install_mcp.get();
        // Take password out of UI state immediately (empty for preview / NOPASSWD).
        let mut sudo_password = self.state.sudo_password.get();
        self.state.sudo_password.set(String::new());
        let stacks = stacks_csv(
            self.state.stack_dockge.get(),
            self.state.stack_open_webui.get(),
            self.state.stack_evcc.get(),
            self.state.stack_whisper.get(),
            self.state.stack_deepseek.get(),
            self.state.stack_piper.get(),
            self.state.stack_openwakeword.get(),
        );
        set_mirrored_log(
            self.state.remote_log,
            if apply {
                "Starting install…".into()
            } else {
                "Starting install preview…".into()
            },
        );
        let remote_log = self.state.remote_log;
        let pending_api_token = self.state.pending_api_token;
        let token_confirm_open = self.state.token_confirm_open;
        let on_refresh = self.on_refresh;
        let status_busy = self.busy;
        spawn_busy(self.state.remote_busy, async move {
            let result = invoke_remote_setup(&RemoteSetupInvokeArgs {
                host,
                install_ssh_key,
                apply,
                release_tag,
                install_status_api,
                install_mcp,
                stacks,
                allow_stale_cli: allow_stale,
                sudo_password: std::mem::take(&mut sudo_password),
            })
            .await;
            wipe_secret_string(&mut sudo_password);
            match result {
                Ok(result) => {
                    append_mirrored_log(remote_log, &result.log);
                    if let Some(api_token) = result.api_token {
                        pending_api_token.set(api_token);
                        token_confirm_open.set(true);
                    }
                    if apply {
                        refresh_status_after_apply(on_refresh, status_busy).await;
                    }
                }
                Err(e) => append_mirrored_log(remote_log, &e),
            }
        });
    }
}

impl Host for ConnectionHost {
    fn get(&self, name: &str) -> Option<Value> {
        let snap = self.snap.get();
        let (label, class) = connection_label(&snap);
        let detail = connection_error_detail(&snap).unwrap_or_default();
        let remote_log = self.state.remote_log.get();
        let apply = self.state.remote_apply.get();
        let hosts = self.state.known_hosts.get();
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
            "sshHost" => Some(Value::Str(self.state.ssh_host.get())),
            "knownHosts" => Some(Value::List(
                hosts.iter().map(|h| Value::Str(h.name.clone())).collect(),
            )),
            "hasKnownHosts" => Some(Value::Bool(!hosts.is_empty())),
            "hostsBusy" => Some(Value::Bool(self.state.hosts_busy.get())),
            "hostsBusyLabel" => Some(Value::Str("Reloading hosts…".into())),
            "hostsHint" => Some(Value::Str(self.state.hosts_hint.get())),
            "installSshKey" => Some(Value::Bool(self.state.install_ssh_key.get())),
            "remoteApply" => Some(Value::Bool(apply)),
            "releaseTag" => Some(Value::Str(self.state.release_tag.get())),
            "releaseTags" => Some(Value::List(
                self.state
                    .release_tags
                    .get()
                    .into_iter()
                    .map(Value::Str)
                    .collect(),
            )),
            "releasesHeading" => {
                let n = self.state.release_tags.get().len();
                Some(Value::Str(format!("Releases ({n})")))
            }
            "installOptionsFlipLabel" => Some(Value::Str(if self.install_options_all_on() {
                "Uncheck all".into()
            } else {
                "Check all".into()
            })),
            "extraAppsFlipLabel" => Some(Value::Str(if self.extra_apps_all_on() {
                "Uncheck all".into()
            } else {
                "Check all".into()
            })),
            "installStatusApi" => Some(Value::Bool(self.state.install_status_api.get())),
            "installMcp" => Some(Value::Bool(self.state.install_mcp.get())),
            "stackDockge" => Some(Value::Bool(self.state.stack_dockge.get())),
            "stackOpenWebui" => Some(Value::Bool(self.state.stack_open_webui.get())),
            "stackEvcc" => Some(Value::Bool(self.state.stack_evcc.get())),
            "stackWhisper" => Some(Value::Bool(self.state.stack_whisper.get())),
            "stackDeepseek" => Some(Value::Bool(self.state.stack_deepseek.get())),
            "stackPiper" => Some(Value::Bool(self.state.stack_piper.get())),
            "stackOpenwakeword" => Some(Value::Bool(self.state.stack_openwakeword.get())),
            "remoteSetupLabel" => Some(Value::Str(if apply {
                "Install on box".into()
            } else {
                "Preview install".into()
            })),
            "remoteBusy" => Some(Value::Bool(self.state.remote_busy.get())),
            "remoteBusyLabel" => Some(Value::Str(if self.state.remote_apply.get() {
                format!("Installing on {}…", self.state.ssh_host.get())
            } else {
                format!("Previewing install for {}…", self.state.ssh_host.get())
            })),
            "remoteLog" => Some(Value::Str(remote_log)),
            "hasRemoteLog" => Some(Value::Bool(!remote_log.is_empty())),
            "showInstallLog" => Some(Value::Bool(
                self.state.remote_busy.get() || !remote_log.is_empty(),
            )),
            "sudoPassword" => Some(Value::Str(self.state.sudo_password.get())),
            "sudoModalOpen" => Some(Value::Bool(self.state.sudo_modal_open.get())),
            "surfacesBusy" => Some(Value::Bool(self.state.surfaces_busy.get())),
            "probeBusyLabel" => {
                let host = self.state.ssh_host.get();
                let host = host.trim();
                if host.is_empty() {
                    Some(Value::Str("Probing…".into()))
                } else {
                    Some(Value::Str(format!("Probing {host}…")))
                }
            }
            "surfacesText" => Some(Value::Str(self.state.surfaces_text.get())),
            "hasSurfacesText" => Some(Value::Bool(!self.state.surfaces_text.get().is_empty())),
            "hasSurfaces" => Some(Value::Bool(self.state.surface_ssh.get() != "?")),
            "surfaceSsh" => Some(Value::Str(self.state.surface_ssh.get())),
            "surfaceCli" => Some(Value::Str(self.state.surface_cli.get())),
            "surfaceApi" => Some(Value::Str(self.state.surface_api.get())),
            "surfaceMcpPc" => Some(Value::Str(self.state.surface_mcp_pc.get())),
            "surfaceMcpBox" => Some(Value::Str(self.state.surface_mcp_box.get())),
            "cliWarnVisible" => Some(Value::Bool(
                self.state.cli_probed.get() && !self.state.cli_current.get(),
            )),
            "allowStaleCli" => Some(Value::Bool(self.state.allow_stale_cli.get())),
            "syncCliBusy" => Some(Value::Bool(self.state.sync_cli_busy.get())),
            "syncCliBusyLabel" => Some(Value::Str(format!(
                "Updating CLI on {}…",
                self.state.ssh_host.get()
            ))),
            "syncCliLog" => Some(Value::Str(self.state.sync_cli_log.get())),
            "hasSyncCliLog" => Some(Value::Bool(!self.state.sync_cli_log.get().is_empty())),
            "applyConfirmOpen" => Some(Value::Bool(self.state.apply_confirm_open.get())),
            "tokenConfirmOpen" => Some(Value::Bool(self.state.token_confirm_open.get())),
            "installLocked" => {
                let probed = self.state.cli_probed.get();
                let current = self.state.cli_current.get();
                let allow = self.state.allow_stale_cli.get();
                Some(Value::Bool(!probed || (!current && !allow)))
            }
            "setupDisabled" => {
                let busy = self.state.remote_busy.get();
                let probed = self.state.cli_probed.get();
                let current = self.state.cli_current.get();
                let allow = self.state.allow_stale_cli.get();
                Some(Value::Bool(
                    busy || !probed || (probed && !current && !allow),
                ))
            }
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
                    self.state.ssh_host.set(s.to_owned());
                    crate::save_ssh_host(s);
                    clear_stale_host_prompts(self.state.surfaces_text, self.state.remote_log);
                    self.state.sync_cli_log.set(String::new());
                    self.state.cli_probed.set(false);
                    self.state.cli_current.set(false);
                    self.state.allow_stale_cli.set(false);
                    self.state.box_cli_version.set("?".into());
                    self.state.surface_ssh.set("?".into());
                    self.state.surface_cli.set("?".into());
                }
                "releaseTag" => self.state.release_tag.set(s.to_owned()),
                "sudoPassword" => self.state.sudo_password.set(s.to_owned()),
                _ => {}
            }
        }
        if let Some(b) = value.as_bool() {
            match name {
                "installSshKey" => self.state.install_ssh_key.set(b),
                "remoteApply" => self.state.remote_apply.set(b),
                "installStatusApi" => self.state.install_status_api.set(b),
                "installMcp" => self.state.install_mcp.set(b),
                "stackDockge" => self.state.stack_dockge.set(b),
                "stackOpenWebui" => self.state.stack_open_webui.set(b),
                "stackEvcc" => self.state.stack_evcc.set(b),
                "stackWhisper" => self.state.stack_whisper.set(b),
                "stackDeepseek" => self.state.stack_deepseek.set(b),
                "stackPiper" => self.state.stack_piper.set(b),
                "stackOpenwakeword" => self.state.stack_openwakeword.set(b),
                "allowStaleCli" => self.state.allow_stale_cli.set(b),
                _ => {}
            }
        }
        Ok(())
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, HostError> {
        if name == "flipInstallOptions" {
            let next = !self.install_options_all_on();
            self.state.install_ssh_key.set(next);
            self.state.install_status_api.set(next);
            self.state.install_mcp.set(next);
            return Ok(Value::Unit);
        }
        if name == "flipExtraApps" {
            let next = !self.extra_apps_all_on();
            self.state.stack_dockge.set(next);
            self.state.stack_open_webui.set(next);
            self.state.stack_evcc.set(next);
            self.state.stack_whisper.set(next);
            self.state.stack_deepseek.set(next);
            self.state.stack_piper.set(next);
            self.state.stack_openwakeword.set(next);
            return Ok(Value::Unit);
        }
        if name == "refresh" && !self.busy.get_untracked() {
            self.on_refresh.run(());
        }
        if name == "reloadHosts" {
            reload_known_hosts(
                self.state.known_hosts,
                self.state.hosts_hint,
                self.state.hosts_busy,
            );
        }
        if name == "pickHost" {
            if let Some(host) = args.first().and_then(Value::as_str) {
                app_log_info(&format!("Host selected: {}", host.trim()));
                apply_picked_host(
                    host,
                    self.state.ssh_host,
                    self.url,
                    self.state.surfaces_text,
                    self.state.remote_log,
                );
                self.state.cli_probed.set(false);
                self.state.cli_current.set(false);
                self.state.allow_stale_cli.set(false);
                self.state.sync_cli_log.set(String::new());
                self.state.box_cli_version.set("?".into());
                self.state.surface_ssh.set("?".into());
                self.state.surface_cli.set("?".into());
                if !self.busy.get_untracked() {
                    self.on_refresh.run(());
                }
            }
        }
        if name == "pickRelease" {
            if let Some(tag) = args.first().and_then(Value::as_str) {
                let tag = tag.trim();
                if !tag.is_empty() {
                    self.state.release_tag.set(tag.to_owned());
                }
            }
        }
        if name == "probeSurfaces" {
            let host = self.state.ssh_host.get().trim().to_owned();
            if host.is_empty() {
                let msg = "Pick or enter a host first.".to_owned();
                app_log_info(&msg);
                self.state.surfaces_text.set(msg);
                return Ok(Value::Unit);
            }
            app_log_info(&format!("Probing {host}…"));
            self.state.surfaces_text.set(String::new());
            let surfaces_text = self.state.surfaces_text;
            let surface_ssh = self.state.surface_ssh;
            let surface_cli = self.state.surface_cli;
            let surface_api = self.state.surface_api;
            let surface_mcp_pc = self.state.surface_mcp_pc;
            let surface_mcp_box = self.state.surface_mcp_box;
            let tip_cli_version = self.state.tip_cli_version;
            let box_cli_version = self.state.box_cli_version;
            let cli_current = self.state.cli_current;
            let cli_probed = self.state.cli_probed;
            let allow_stale_cli = self.state.allow_stale_cli;
            spawn_busy(self.state.surfaces_busy, async move {
                match invoke_remote_surfaces(&host).await {
                    Ok(report) => {
                        surface_ssh.set(report.ssh.clone());
                        surface_api.set(report.api.clone());
                        surface_mcp_pc.set(report.mcp_pc.clone());
                        surface_mcp_box.set(report.mcp_box.clone());
                        app_log_info(&report.text);
                        surfaces_text.set(report.text);
                        let tip = if report.tip_version.is_empty() {
                            tip_cli_version.get_untracked()
                        } else {
                            tip_cli_version.set(report.tip_version.clone());
                            report.tip_version
                        };
                        box_cli_version.set(report.box_cli.clone());
                        cli_current.set(report.cli_current);
                        cli_probed.set(true);
                        if report.cli_current {
                            allow_stale_cli.set(false);
                        }
                        surface_cli.set(format_cli_surface_label(
                            &report.box_cli,
                            &tip,
                            report.cli_current,
                        ));
                    }
                    Err(e) => {
                        surface_ssh.set("?".into());
                        surface_cli.set("?".into());
                        surface_api.set("?".into());
                        surface_mcp_pc.set("?".into());
                        surface_mcp_box.set("?".into());
                        app_log_error(&e);
                        surfaces_text.set(e);
                        box_cli_version.set("?".into());
                        cli_current.set(false);
                        cli_probed.set(true);
                    }
                }
            });
        }
        if name == "syncCli" {
            let host = self.state.ssh_host.get().trim().to_owned();
            if host.is_empty() {
                let msg = "Pick or enter a host first.".to_owned();
                app_log_info(&msg);
                self.state.sync_cli_log.set(msg);
                return Ok(Value::Unit);
            }
            app_log_info(&format!("Updating CLI on {host}…"));
            let install_ssh_key = self.state.install_ssh_key.get();
            let release_tag = self.state.release_tag.get().trim().to_owned();
            let release_tag = if release_tag.is_empty() {
                "dev-preview".to_owned()
            } else {
                release_tag
            };
            let tip_cli_version = self.state.tip_cli_version;
            let box_cli_version = self.state.box_cli_version;
            let cli_current = self.state.cli_current;
            let cli_probed = self.state.cli_probed;
            let allow_stale_cli = self.state.allow_stale_cli;
            let surface_cli = self.state.surface_cli;
            let sync_cli_log = self.state.sync_cli_log;
            sync_cli_log.set(String::new());
            spawn_busy(self.state.sync_cli_busy, async move {
                match invoke_remote_upload_cli(&host, install_ssh_key, &release_tag).await {
                    Ok(probe) => {
                        if !probe.tip_version.is_empty() {
                            tip_cli_version.set(probe.tip_version.clone());
                        }
                        let label = probe.version.clone().unwrap_or_else(|| probe.label.clone());
                        let tip = tip_cli_version.get_untracked();
                        box_cli_version.set(label.clone());
                        cli_current.set(probe.current);
                        cli_probed.set(true);
                        if probe.current {
                            allow_stale_cli.set(false);
                        }
                        surface_cli.set(format_cli_surface_label(&label, &tip, probe.current));
                        let msg = if probe.current {
                            format!("CLI on box updated to {label}.")
                        } else {
                            format!("CLI uploaded ({label}); still behind Desktop {tip}.")
                        };
                        app_log_info(&msg);
                        sync_cli_log.set(msg);
                    }
                    Err(e) => {
                        app_log_error(&e);
                        sync_cli_log.set(e);
                    }
                }
            });
        }
        if name == "remoteSetup" {
            if let Err(msg) = self.remote_setup_preflight() {
                set_mirrored_log(self.state.remote_log, msg);
                return Ok(Value::Unit);
            }
            if self.state.remote_apply.get() {
                self.state.apply_confirm_open.set(true);
                return Ok(Value::Unit);
            }
            self.begin_remote_setup();
        }
        if name == "confirmApply" {
            self.state.apply_confirm_open.set(false);
            if let Err(msg) = self.remote_setup_preflight() {
                set_mirrored_log(self.state.remote_log, msg);
                return Ok(Value::Unit);
            }
            self.state.sudo_password.set(String::new());
            self.state.sudo_modal_open.set(true);
        }
        if name == "cancelApply" {
            self.state.apply_confirm_open.set(false);
            clear_sudo_field(self.state);
            set_mirrored_log(self.state.remote_log, "Remote apply cancelled.".into());
        }
        if name == "submitSudo" {
            self.state.sudo_modal_open.set(false);
            if let Err(msg) = self.remote_setup_preflight() {
                clear_sudo_field(self.state);
                set_mirrored_log(self.state.remote_log, msg);
                return Ok(Value::Unit);
            }
            self.begin_remote_setup();
        }
        if name == "cancelSudo" {
            self.state.sudo_modal_open.set(false);
            clear_sudo_field(self.state);
            set_mirrored_log(self.state.remote_log, "Remote apply cancelled.".into());
        }
        if name == "confirmSaveToken" {
            let api_token = self.state.pending_api_token.get();
            self.state.pending_api_token.set(String::new());
            self.state.token_confirm_open.set(false);
            if !api_token.is_empty() {
                self.token.set(api_token.clone());
                crate::save_api_token(&api_token);
                let mut log = self.state.remote_log.get();
                if !log.is_empty() {
                    log.push('\n');
                }
                log.push_str("Saved status-api bearer into Connection.");
                set_mirrored_log(self.state.remote_log, log);
            }
        }
        if name == "cancelSaveToken" {
            self.state.pending_api_token.set(String::new());
            self.state.token_confirm_open.set(false);
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
    crate::save_ssh_host(host);
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

fn clear_sudo_field(state: ConnectionState) {
    let mut leftover = state.sudo_password.get();
    wipe_secret_string(&mut leftover);
    state.sudo_password.set(String::new());
}

fn wipe_secret_string(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
    drop(bytes);
}

const INSTALL_LOG_MAX_CHARS: usize = 24_000;

fn set_mirrored_log(signal: RwSignal<String>, text: String) {
    app_log_info(&text);
    signal.set(text);
}

fn append_mirrored_log(signal: RwSignal<String>, text: &str) {
    let text = text.trim_end();
    if text.is_empty() {
        return;
    }
    app_log_info(text);
    signal.update(|cur| {
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(text);
        trim_install_log(cur);
    });
    scroll_install_log_to_end();
}

fn append_install_progress(remote_log: RwSignal<String>, line: &str) {
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }
    // Already in the Desktop log ring via tracing; only mirror into the install panel.
    remote_log.update(|cur| {
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        trim_install_log(cur);
    });
    scroll_install_log_to_end();
}

fn trim_install_log(buf: &mut String) {
    if buf.len() <= INSTALL_LOG_MAX_CHARS {
        return;
    }
    let excess = buf.len() - INSTALL_LOG_MAX_CHARS;
    let drop_at = buf[excess..].find('\n').map_or(excess, |i| excess + i + 1);
    let kept = buf.split_off(drop_at);
    *buf = kept;
    if !buf.starts_with('…') {
        buf.insert_str(0, "…\n");
    }
}

fn scroll_install_log_to_end() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(el)) = document.query_selector(".connection__install-log-body") {
        if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
            el.set_scroll_top(el.scroll_height());
        }
    }
}

fn attach_install_log_listener(state: ConnectionState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::Event| {
        if !state.remote_busy.get_untracked() {
            return;
        }
        let Some(custom) = event.dyn_ref::<web_sys::CustomEvent>() else {
            return;
        };
        let detail = custom.detail();
        let message = js_sys::Reflect::get(&detail, &"message".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        if message.is_empty() {
            return;
        }
        // Progress banners from the remote runner (`[horto remote] …`).
        if message.contains("[horto remote]") || message.starts_with("Starting install") {
            append_install_progress(state.remote_log, &message);
        }
    }) as Box<dyn FnMut(_)>);
    let _ = window.add_event_listener_with_callback("horto-log", closure.as_ref().unchecked_ref());
    closure.forget();
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
    app_log_info("Reloading known hosts…");
    if !is_desktop_shell() {
        known_hosts.set(Vec::new());
        set_hosts_hint(hosts_hint, "Host list needs the Desktop app.");
        app_log_info("Host list needs the Desktop app.");
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
                let msg = "Reload hosts timed out. Try again.";
                set_hosts_hint(hint_watch, msg);
                app_log_error(msg);
                busy_watch.set(false);
            }
        });
        match invoke_list_known_remote_hosts().await {
            Ok(list) => {
                let n = list.len();
                set_hosts_hint(hosts_hint, &hosts_picker_hint(n));
                known_hosts.set(list);
                app_log_info(&format!("Loaded {n} host(s)."));
            }
            Err(e) => {
                known_hosts.set(Vec::new());
                let msg = format!("Could not load hosts: {e}");
                set_hosts_hint(hosts_hint, &msg);
                app_log_error(&msg);
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

fn reload_tip_cli_version(tip_cli_version: RwSignal<String>) {
    if !is_desktop_shell() {
        return;
    }
    leptos::task::spawn_local(async move {
        if let Ok(v) = invoke_tip_cli_version().await {
            if !v.is_empty() {
                tip_cli_version.set(v);
            }
        }
    });
}

/// One CLI row: box version alone when current; otherwise append Desktop tip.
fn format_cli_surface_label(box_ver: &str, tip: &str, current: bool) -> String {
    let box_ver = box_ver.trim();
    if box_ver.is_empty() || box_ver == "?" {
        return "?".into();
    }
    let tip = tip.trim();
    if current || tip.is_empty() {
        return box_ver.to_owned();
    }
    format!("{box_ver} · Desktop {tip}")
}

/// Desktop vs box long-version match (same rules as shared `remote_cli_version_is_current`).
fn cli_versions_match(tip: &str, box_ver: &str) -> bool {
    let tip = tip.trim();
    let box_ver = box_ver.trim();
    if tip.is_empty() || box_ver.is_empty() {
        return false;
    }
    box_ver == tip || box_ver == format!("horto-os-ui {tip}")
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
    api: String,
    mcp_pc: String,
    mcp_box: String,
    tip_version: String,
    box_cli: String,
    cli_current: bool,
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

    let cli_current = Reflect::get(&cli_obj, &"current".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let box_cli = {
        let from_version = Reflect::get(&cli_obj, &"version".into())
            .ok()
            .and_then(|v| {
                if v.is_null() || v.is_undefined() {
                    None
                } else {
                    v.as_string().filter(|s| !s.is_empty())
                }
            });
        from_version.unwrap_or_else(|| cli_status.clone())
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
        api: format!("{api_health} ({api_url})"),
        mcp_pc: mcp_pc_summary,
        mcp_box: mcp_box_summary,
        tip_version: local,
        box_cli,
        cli_current,
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

struct RemoteSetupInvokeArgs {
    host: String,
    install_ssh_key: bool,
    apply: bool,
    release_tag: String,
    install_status_api: bool,
    install_mcp: bool,
    stacks: String,
    allow_stale_cli: bool,
    sudo_password: String,
}

async fn invoke_remote_setup(args: &RemoteSetupInvokeArgs) -> Result<RemoteSetupUiResult, String> {
    let payload = Object::new();
    Reflect::set(&payload, &"host".into(), &args.host.as_str().into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"installSshKey".into(),
        &args.install_ssh_key.into(),
    )
    .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"apply".into(), &args.apply.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"releaseTag".into(),
        &args.release_tag.as_str().into(),
    )
    .map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"installStatusApi".into(),
        &args.install_status_api.into(),
    )
    .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"installMcp".into(), &args.install_mcp.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"stacks".into(), &args.stacks.as_str().into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"full".into(), &true.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"allowStaleCli".into(),
        &args.allow_stale_cli.into(),
    )
    .map_err(|e| format!("{e:?}"))?;
    Reflect::set(
        &payload,
        &"sudoPassword".into(),
        &args.sudo_password.as_str().into(),
    )
    .map_err(|e| format!("{e:?}"))?;

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

struct RemoteUploadUi {
    version: Option<String>,
    current: bool,
    label: String,
    tip_version: String,
}

async fn invoke_remote_upload_cli(
    host: &str,
    install_ssh_key: bool,
    release_tag: &str,
) -> Result<RemoteUploadUi, String> {
    let payload = Object::new();
    Reflect::set(&payload, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"installSshKey".into(), &install_ssh_key.into())
        .map_err(|e| format!("{e:?}"))?;
    Reflect::set(&payload, &"releaseTag".into(), &release_tag.into())
        .map_err(|e| format!("{e:?}"))?;
    let value = crate::tauri_bridge::invoke_remote_upload_cli_cmd(&payload).await?;
    let version = Reflect::get(&value, &"version".into()).ok().and_then(|v| {
        if v.is_null() || v.is_undefined() {
            None
        } else {
            v.as_string().filter(|s| !s.is_empty())
        }
    });
    let current = Reflect::get(&value, &"current".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let label = Reflect::get(&value, &"label".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "?".into());
    let tip_version = Reflect::get(&value, &"tipVersion".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    Ok(RemoteUploadUi {
        version,
        current,
        label,
        tip_version,
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

/// After Apply, status-api may still be restarting; refresh once then retry.
#[allow(clippy::future_not_send)]
async fn refresh_status_after_apply(on_refresh: Callback<()>, status_busy: RwSignal<bool>) {
    const DELAYS_MS: &[u32] = &[0, 1_500, 3_000];
    for (i, delay) in DELAYS_MS.iter().enumerate() {
        if *delay > 0 {
            gloo_timers::future::TimeoutFuture::new(*delay).await;
        }
        wait_until_status_idle(status_busy).await;
        on_refresh.run(());
        if i + 1 < DELAYS_MS.len() {
            wait_until_status_idle(status_busy).await;
        }
    }
}

#[allow(clippy::future_not_send)]
async fn wait_until_status_idle(status_busy: RwSignal<bool>) {
    for _ in 0..80 {
        if !status_busy.get_untracked() {
            return;
        }
        gloo_timers::future::TimeoutFuture::new(100).await;
    }
}
