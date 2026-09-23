use leptos::prelude::*;

use crate::components::{
    boot_connection, BoxStatusPanel, ConnectionPanel, ConnectionState, ContainersPanel,
    ServicesPanel, TopBarPanel,
};
use crate::menu_bridge::attach_menu_bridge;
use crate::status::{fetch_snapshot, merge_urls_with_catalog, normalize_bearer_token, Snapshot};
use crate::{
    align_status_api_url_to_hostname, apply_theme, build_footer, default_api_token,
    default_status_api_url, default_theme, hydrate_api_token, save_status_api_url, Screen,
};

const MIN_BUSY_MS: f64 = 550.0;

#[component]
pub fn App() -> impl IntoView {
    let url = RwSignal::new(default_status_api_url());
    let token = RwSignal::new(default_api_token());
    let screen = RwSignal::new(Screen::Connection);
    let theme = RwSignal::new(default_theme());
    let busy = RwSignal::new(false);
    let about_open = RwSignal::new(false);
    let snap = RwSignal::new(Snapshot {
        health_ok: None,
        status: None,
        api_cli_version: None,
        error: None,
    });
    // App-owned: survives Connection panel remount on tab change.
    let connection = ConnectionState::new();
    let started = StoredValue::new(false);
    let menu_attached = StoredValue::new(false);

    Effect::new(move |_| apply_theme(&theme.get()));

    let do_refresh = Callback::new(move |()| {
        if busy.get_untracked() {
            return;
        }
        let base = url.get();
        save_status_api_url(&base);
        busy.set(true);
        leptos::task::spawn_local(async move {
            run_status_refresh(base, url, token, snap, busy).await;
        });
    });

    Effect::new(move |_| {
        if !menu_attached.get_value() {
            menu_attached.set_value(true);
            attach_menu_bridge(screen, theme, do_refresh, about_open);
        }
    });

    Effect::new(move |_| {
        boot_connection(connection);
    });

    Effect::new(move |_| {
        if !started.get_value() {
            started.set_value(true);
            hydrate_api_token(token, url, snap, do_refresh);
        }
    });

    view! {
        <div class="app">
            <TopBarPanel screen=screen theme=theme snap=snap busy=busy on_refresh=do_refresh />
            <main class="shell">
                <Show when=move || screen.get() == Screen::Connection fallback=|| ()>
                    <ConnectionPanel
                        url=url
                        token=token
                        busy=busy
                        snap=snap
                        on_refresh=do_refresh
                        state=connection
                    />
                </Show>
                <Show when=move || screen.get() == Screen::Overview fallback=|| ()>
                    {move || overview_panels(url, token, snap, do_refresh)}
                </Show>
                <Show when=move || screen.get() == Screen::Services fallback=|| ()>
                    {move || services_panel(url, snap)}
                </Show>
                <p class="footer-note">{move || build_footer()}</p>
            </main>
        </div>
        <Show when=move || about_open.get() fallback=|| ()>
            {about_dialog(about_open)}
        </Show>
    }
}

fn overview_panels(
    url: RwSignal<String>,
    token: RwSignal<String>,
    snap: RwSignal<Snapshot>,
    on_refresh: Callback<()>,
) -> AnyView {
    let api = url.get();
    let st = match snap.get().status {
        Some(st) => st,
        // No Status API: still show Overview chrome (unknown metrics, empty containers).
        None => offline_box_status(&api),
    };
    let api_containers = api.clone();
    view! {
        <BoxStatusPanel status=st.clone() api_base=api token=token on_refresh=on_refresh />
        <ContainersPanel containers=st.containers urls=st.urls api_base=api_containers />
    }
    .into_any()
}

fn offline_box_status(api_base: &str) -> crate::status::BoxStatus {
    let host = host_hint_from_api_base(api_base);
    crate::status::BoxStatus {
        hostname: host,
        urls: merge_urls_with_catalog(Vec::new(), api_base),
        ..Default::default()
    }
}

fn host_hint_from_api_base(api_base: &str) -> String {
    let s = api_base.trim();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or("");
    let host_port = rest.split('/').next().unwrap_or("");
    let host = host_port
        .rsplit_once('@')
        .map_or(host_port, |(_, h)| h)
        .rsplit_once(':')
        .map_or(host_port, |(h, _)| h);
    if host.is_empty() {
        "unknown".into()
    } else {
        host.to_owned()
    }
}

fn services_panel(url: RwSignal<String>, snap: RwSignal<Snapshot>) -> AnyView {
    let api = url.get();
    let urls = match snap.get().status {
        Some(st) => st.urls,
        // No Status API: still show the full tip catalog (defaults, down).
        None => merge_urls_with_catalog(Vec::new(), &api),
    };
    view! { <ServicesPanel urls=urls api_base=api /> }.into_any()
}

fn about_dialog(about_open: RwSignal<bool>) -> AnyView {
    view! {
        <div class="about-backdrop" on:click=move |_| about_open.set(false)>
            <div class="about-dialog" role="dialog" aria-labelledby="about-title" on:click=move |ev| ev.stop_propagation()>
                <img class="about-logo" src="/hortos-logo.png" width="56" height="56" alt="" />
                <h2 id="about-title">"About Horto"</h2>
                <p>"Homeowner desktop for your Horto box."</p>
                <p class="about-meta">{move || {
                    format!("horto-os-ui · Tauri 2 · Leptos · {}", build_footer())
                }}</p>
                <button type="button" class="about-close" on:click=move |_| about_open.set(false)>
                    "Close"
                </button>
            </div>
        </div>
    }
    .into_any()
}

#[allow(clippy::future_not_send)]
async fn run_status_refresh(
    base: String,
    url: RwSignal<String>,
    token: RwSignal<String>,
    snap: RwSignal<Snapshot>,
    busy: RwSignal<bool>,
) {
    let tok = match resolve_bearer(token).await {
        Ok(t) => t,
        Err(e) => {
            snap.set(Snapshot {
                health_ok: None,
                status: None,
                api_cli_version: None,
                error: Some(e),
            });
            busy.set(false);
            return;
        }
    };
    if !tok.is_empty() {
        apply_token_signal(token, &tok);
    }
    let started = js_sys::Date::now();
    let next = fetch_snapshot(base.clone(), (!tok.is_empty()).then_some(tok)).await;
    apply_snapshot_with_align(base, url, token, snap, next).await;
    let elapsed = js_sys::Date::now() - started;
    if elapsed < MIN_BUSY_MS {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wait_ms = (MIN_BUSY_MS - elapsed) as u32;
        gloo_timers::future::TimeoutFuture::new(wait_ms).await;
    }
    busy.set(false);
}

/// Bearer for Status API: tip file on Desktop; Connection field only in the browser.
#[allow(clippy::future_not_send)]
async fn resolve_bearer(token: RwSignal<String>) -> Result<String, String> {
    match crate::tauri_bridge::invoke_read_api_token().await {
        Ok(disk) => {
            let disk = normalize_bearer_token(&disk);
            if disk.is_empty() && crate::tauri_bridge::is_desktop_shell() {
                Err(
                    "Status API token file is empty. Save the box token with TUI or CLI, or point the Status API URL at the box and restart Horto."
                        .into(),
                )
            } else if disk.is_empty() {
                Ok(normalize_bearer_token(&token.get()))
            } else {
                Ok(disk)
            }
        }
        Err(e) if e.contains("desktop shell") => Ok(normalize_bearer_token(&token.get())),
        Err(e) => Err(format!("Could not read the Status API token file: {e}")),
    }
}

fn apply_token_signal(token: RwSignal<String>, value: &str) {
    token.set(normalize_bearer_token(value));
}

#[allow(clippy::future_not_send)]
async fn apply_snapshot_with_align(
    base: String,
    url: RwSignal<String>,
    token: RwSignal<String>,
    snap: RwSignal<Snapshot>,
    next: Snapshot,
) {
    let Some(st) = next.status.as_ref() else {
        snap.set(next);
        return;
    };
    let Some(aligned) = align_status_api_url_to_hostname(&base, &st.hostname) else {
        snap.set(next);
        return;
    };
    let aligned_tok = token.get();
    let aligned_opt = (!aligned_tok.is_empty()).then_some(aligned_tok);
    let aligned_snap = fetch_snapshot(aligned.clone(), aligned_opt).await;
    if aligned_snap.error.is_none() {
        url.set(aligned.clone());
        save_status_api_url(&aligned);
        snap.set(aligned_snap);
    } else {
        snap.set(next);
    }
}
