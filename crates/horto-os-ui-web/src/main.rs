//! Horto OS UI web (Leptos CSR): status panels for the Tauri desktop shell
//! and browser Trunk builds. Deserializes shared status JSON; no setup apply.

use leptos::mount::mount_to_body;

mod app;
mod components;
mod menu_bridge;
mod status;

use app::App;

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

pub fn save_api_token(token: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("horto_api_token", token);
    }
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
