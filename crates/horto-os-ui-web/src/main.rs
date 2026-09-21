//! Horto OS UI web (Leptos CSR): status panels for the Tauri desktop shell
//! and browser Trunk builds. Deserializes shared status JSON; no setup apply.

use leptos::mount::mount_to_body;

mod app;
mod components;
mod menu_bridge;
mod status;
mod tauri_bridge;

use app::App;
use leptos::prelude::*;
use tauri_bridge::{invoke_read_api_token, invoke_sync_api_token_from_box, invoke_write_api_token};

/// Package version and short git SHA from `build.rs`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_COMMIT: &str = env!("GIT_COMMIT_HASH");

/// Footer / About line: `v0.1.0 (abc1234)`.
pub fn build_footer() -> String {
    format!("v{VERSION} ({GIT_COMMIT})")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Connection,
    Services,
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

pub fn default_status_api_url() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("horto_status_api_url").ok().flatten())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://localhost:8787".into())
}

pub fn save_status_api_url(url: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("horto_status_api_url", url);
    }
}

pub fn default_api_token() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("horto_api_token").ok().flatten())
        .unwrap_or_default()
}

/// Mirror into localStorage and, in Desktop, the shared tip file used by TUI / CLI.
pub fn save_api_token(token: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            let _ = storage.remove_item("horto_api_token");
        } else {
            let _ = storage.set_item("horto_api_token", trimmed);
        }
    }
    let token = token.trim().to_owned();
    if token.is_empty() {
        return;
    }
    leptos::task::spawn_local(async move {
        let _ = invoke_write_api_token(&token).await;
    });
}

/// Prefer a usable tip file, then (non-loopback Status API host) pull over SSH.
pub fn hydrate_api_token(
    token: RwSignal<String>,
    url: RwSignal<String>,
    snap: RwSignal<crate::status::Snapshot>,
    on_ready: Callback<()>,
) {
    leptos::task::spawn_local(async move {
        let mut loaded = String::new();
        if let Ok(Some(disk)) = invoke_read_api_token().await {
            disk.trim().clone_into(&mut loaded);
        }
        let sync_err = if loaded.is_empty() {
            if let Some(host) = sync_host_from_status_api_url(&url.get()) {
                match invoke_sync_api_token_from_box(&host).await {
                    Ok(box_tok) => {
                        loaded = box_tok;
                        None
                    }
                    Err(e) if e.contains("desktop shell") => None,
                    Err(e) => Some(e),
                }
            } else {
                None
            }
        } else {
            None
        };
        if !loaded.is_empty() {
            token.set(loaded.clone());
            if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            {
                let _ = storage.set_item("horto_api_token", &loaded);
            }
        }
        if let Some(err) = sync_err {
            snap.set(crate::status::Snapshot {
                health_ok: None,
                status: None,
                error: Some(format!(
                    "Could not load the Status API token from the box: {err}"
                )),
            });
        }
        on_ready.run(());
    });
}

/// OpenSSH Host / hostname taken from a non-loopback Status API URL.
#[must_use]
pub fn sync_host_from_status_api_url(url: &str) -> Option<String> {
    let parsed = web_sys::Url::new(url.trim()).ok()?;
    let host = parsed.hostname();
    if host.is_empty() || is_loopback_hostname(&host) {
        None
    } else {
        Some(host)
    }
}

/// True when a snapshot error reports HTTP 401 Unauthorized.
#[must_use]
pub fn snapshot_is_unauthorized(snap: &crate::status::Snapshot) -> bool {
    snap.error
        .as_deref()
        .is_some_and(|e| e.contains("HTTP 401"))
}

/// Retarget a loopback Status API URL to the box hostname after a successful fetch.
#[must_use]
pub fn align_status_api_url_to_hostname(url: &str, hostname: &str) -> Option<String> {
    let hostname = hostname.trim();
    if hostname.is_empty() || hostname.eq_ignore_ascii_case("unknown") {
        return None;
    }
    let parsed = web_sys::Url::new(url).ok()?;
    let current = parsed.hostname();
    if !is_loopback_hostname(&current) {
        return None;
    }
    parsed.set_hostname(hostname);
    let aligned = parsed.href();
    if aligned == url {
        None
    } else {
        Some(aligned)
    }
}

fn is_loopback_hostname(host: &str) -> bool {
    let h = host.trim().trim_matches(|c| c == '[' || c == ']');
    h.eq_ignore_ascii_case("localhost") || h == "127.0.0.1" || h == "::1" || h == "0.0.0.0"
}

/// Stored theme: `system` | `light` | `dark`.
pub fn default_theme() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("horto_theme").ok().flatten())
        .filter(|t| t == "light" || t == "dark" || t == "system")
        .unwrap_or_else(|| "system".into())
}

pub fn apply_theme(theme: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = document.document_element() {
            match theme {
                "light" | "dark" => {
                    let _ = el.set_attribute("data-theme", theme);
                }
                _ => {
                    let _ = el.remove_attribute("data-theme");
                }
            }
        }
    }
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("horto_theme", theme);
    }
}

/// Cycle system → light → dark → system.
pub fn cycle_theme(current: &str) -> String {
    match current {
        "light" => "dark".into(),
        "dark" => "system".into(),
        _ => "light".into(),
    }
}
