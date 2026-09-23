//! Durable Connection tab state owned by [`crate::app::App`].
//!
//! Survives Connection panel remount when the user switches Overview / Services
//! tabs. Probe results, host pick, and install options live here; the panel is
//! only a view over these signals.

use leptos::prelude::*;

use crate::tauri_bridge::KnownHostUi;

/// App-owned Connection signals (host, probes, remote install).
///
/// Construct once in [`crate::app::App`] and pass into [`super::ConnectionPanel`].
#[derive(Clone, Copy)]
pub struct ConnectionState {
    /// SSH / probe host alias or `user@host`.
    pub ssh_host: RwSignal<String>,
    /// Desktop-known remote hosts for the picker.
    pub known_hosts: RwSignal<Vec<KnownHostUi>>,
    /// Host list reload in flight.
    pub hosts_busy: RwSignal<bool>,
    /// Short hint under the host picker.
    pub hosts_hint: RwSignal<String>,
    /// Install SSH key during remote setup.
    pub install_ssh_key: RwSignal<bool>,
    /// When true, remote setup applies; when false, plan only.
    pub remote_apply: RwSignal<bool>,
    /// Selected GitHub release tag for remote install.
    pub release_tag: RwSignal<String>,
    /// Available release tags from Desktop.
    pub release_tags: RwSignal<Vec<String>>,
    /// Install status-api on the box.
    pub install_status_api: RwSignal<bool>,
    /// Install MCP on the box.
    pub install_mcp: RwSignal<bool>,
    /// Extra stack: Dockge.
    pub stack_dockge: RwSignal<bool>,
    /// Extra stack: Open WebUI.
    pub stack_open_webui: RwSignal<bool>,
    /// Extra stack: EVCC.
    pub stack_evcc: RwSignal<bool>,
    /// Extra stack: Whisper.
    pub stack_whisper: RwSignal<bool>,
    /// Extra stack: DeepSeek.
    pub stack_deepseek: RwSignal<bool>,
    /// Extra stack: Piper.
    pub stack_piper: RwSignal<bool>,
    /// Extra stack: OpenWakeWord.
    pub stack_openwakeword: RwSignal<bool>,
    /// Remote install / preview in flight.
    pub remote_busy: RwSignal<bool>,
    /// Remote install log text.
    pub remote_log: RwSignal<String>,
    /// Surface probe in flight.
    pub surfaces_busy: RwSignal<bool>,
    /// Full surface probe report text.
    pub surfaces_text: RwSignal<String>,
    /// SSH surface chip.
    pub surface_ssh: RwSignal<String>,
    /// CLI surface chip.
    pub surface_cli: RwSignal<String>,
    /// API surface chip.
    pub surface_api: RwSignal<String>,
    /// MCP (PC) surface chip.
    pub surface_mcp_pc: RwSignal<String>,
    /// MCP (box) surface chip.
    pub surface_mcp_box: RwSignal<String>,
    /// Desktop tip CLI long-version.
    pub tip_cli_version: RwSignal<String>,
    /// Box CLI long-version from probe or Status API.
    pub box_cli_version: RwSignal<String>,
    /// True when box CLI matches Desktop tip.
    pub cli_current: RwSignal<bool>,
    /// True after at least one CLI probe / API version sync.
    pub cli_probed: RwSignal<bool>,
    /// Allow install when box CLI is behind tip.
    pub allow_stale_cli: RwSignal<bool>,
    /// Sync CLI upload in flight.
    pub sync_cli_busy: RwSignal<bool>,
    /// Sync CLI log text.
    pub sync_cli_log: RwSignal<String>,
    /// Apply-confirm modal open.
    pub apply_confirm_open: RwSignal<bool>,
    /// Box sudo password for Apply (not persisted).
    pub sudo_password: RwSignal<String>,
    /// Token-confirm modal open.
    pub token_confirm_open: RwSignal<bool>,
    /// Token captured from remote install, awaiting confirm.
    pub pending_api_token: RwSignal<String>,
    /// One-shot boot (hosts / tags / tip version) completed.
    booted: StoredValue<bool>,
}

impl ConnectionState {
    /// Empty durable Connection state for App.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ssh_host: RwSignal::new(crate::default_ssh_host()),
            known_hosts: RwSignal::new(Vec::new()),
            hosts_busy: RwSignal::new(false),
            hosts_hint: RwSignal::new(String::from(
                "Pick where Status API and probes go. localhost is this PC.",
            )),
            install_ssh_key: RwSignal::new(false),
            remote_apply: RwSignal::new(false),
            release_tag: RwSignal::new(String::from("dev-preview")),
            release_tags: RwSignal::new(vec![String::from("dev-preview")]),
            install_status_api: RwSignal::new(true),
            install_mcp: RwSignal::new(true),
            stack_dockge: RwSignal::new(true),
            stack_open_webui: RwSignal::new(false),
            stack_evcc: RwSignal::new(false),
            stack_whisper: RwSignal::new(false),
            stack_deepseek: RwSignal::new(false),
            stack_piper: RwSignal::new(false),
            stack_openwakeword: RwSignal::new(false),
            remote_busy: RwSignal::new(false),
            remote_log: RwSignal::new(String::new()),
            surfaces_busy: RwSignal::new(false),
            surfaces_text: RwSignal::new(String::new()),
            surface_ssh: RwSignal::new(String::from("?")),
            surface_cli: RwSignal::new(String::from("?")),
            surface_api: RwSignal::new(String::from("?")),
            surface_mcp_pc: RwSignal::new(String::from("?")),
            surface_mcp_box: RwSignal::new(String::from("?")),
            tip_cli_version: RwSignal::new(String::new()),
            box_cli_version: RwSignal::new(String::from("?")),
            cli_current: RwSignal::new(false),
            cli_probed: RwSignal::new(false),
            allow_stale_cli: RwSignal::new(false),
            sync_cli_busy: RwSignal::new(false),
            sync_cli_log: RwSignal::new(String::new()),
            apply_confirm_open: RwSignal::new(false),
            sudo_password: RwSignal::new(String::new()),
            token_confirm_open: RwSignal::new(false),
            pending_api_token: RwSignal::new(String::new()),
            booted: StoredValue::new(false),
        }
    }

    /// True when one-shot Desktop boot already ran.
    #[must_use]
    pub fn is_booted(self) -> bool {
        self.booted.get_value()
    }

    /// Mark one-shot Desktop boot done.
    pub fn mark_booted(self) {
        self.booted.set_value(true);
    }
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}
