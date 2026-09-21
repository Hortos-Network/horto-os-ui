use leptos::prelude::*;

use crate::components::{
    BoxStatusPanel, ConnectionPanel, ContainersPanel, ServicesPanel, TopBarPanel,
};
use crate::menu_bridge::attach_menu_bridge;
use crate::status::{fetch_snapshot, Snapshot};
use crate::{
    align_status_api_url_to_hostname, apply_theme, build_footer, default_api_token,
    default_status_api_url, default_theme, hydrate_api_token, save_api_token, save_status_api_url,
    snapshot_is_unauthorized, sync_host_from_status_api_url, Screen,
};

const MIN_BUSY_MS: f64 = 550.0;

#[component]
pub fn App() -> impl IntoView {
    let url = RwSignal::new(default_status_api_url());
    let token = RwSignal::new(default_api_token());
    let screen = RwSignal::new(Screen::Overview);
    let theme = RwSignal::new(default_theme());
    let busy = RwSignal::new(false);
    let about_open = RwSignal::new(false);
    let snap = RwSignal::new(Snapshot {
        health_ok: None,
        status: None,
        error: None,
    });
    let started = StoredValue::new(false);
    let menu_attached = StoredValue::new(false);

    Effect::new(move |_| apply_theme(&theme.get()));

    let do_refresh = Callback::new(move |()| {
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
                    <ConnectionPanel url=url token=token busy=busy snap=snap on_refresh=do_refresh />
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
    let Some(st) = snap.get().status else {
        return ().into_any();
    };
    let api_containers = api.clone();
    view! {
        <BoxStatusPanel status=st.clone() api_base=api token=token on_refresh=on_refresh />
        <ContainersPanel containers=st.containers urls=st.urls api_base=api_containers />
    }
    .into_any()
}

fn services_panel(url: RwSignal<String>, snap: RwSignal<Snapshot>) -> AnyView {
    let api = url.get();
    let Some(st) = snap.get().status else {
        return ().into_any();
    };
    view! { <ServicesPanel urls=st.urls api_base=api /> }.into_any()
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
    let mut tok = token.get().trim().to_owned();
    if tok.is_empty() {
        if let Some(host) = sync_host_from_status_api_url(&base) {
            match ensure_token_from_box(&host).await {
                Ok(box_tok) => {
                    tok = box_tok;
                    apply_token_signal(token, &tok);
                }
                Err(Some(e)) => {
                    snap.set(Snapshot {
                        health_ok: None,
                        status: None,
                        error: Some(e),
                    });
                    busy.set(false);
                    return;
                }
                Err(None) => {}
            }
        }
    }
    if !tok.is_empty() {
        save_api_token(&tok);
    }
    let tok_opt = (!tok.is_empty()).then_some(tok);
    let started = js_sys::Date::now();
    let mut next = fetch_snapshot(base.clone(), tok_opt).await;
    if snapshot_is_unauthorized(&next) {
        if let Some(host) = sync_host_from_status_api_url(&base) {
            match ensure_token_from_box(&host).await {
                Ok(box_tok) => {
                    apply_token_signal(token, &box_tok);
                    save_api_token(&box_tok);
                    next = fetch_snapshot(base.clone(), Some(box_tok)).await;
                }
                Err(Some(e)) => next.error = Some(e),
                Err(None) => {}
            }
        }
    }
    apply_snapshot_with_align(base, url, token, snap, next).await;
    let elapsed = js_sys::Date::now() - started;
    if elapsed < MIN_BUSY_MS {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wait_ms = (MIN_BUSY_MS - elapsed) as u32;
        gloo_timers::future::TimeoutFuture::new(wait_ms).await;
    }
    busy.set(false);
}

fn apply_token_signal(token: RwSignal<String>, value: &str) {
    token.set(value.to_owned());
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("horto_api_token", value);
    }
}

/// `Ok(token)` synced; `Err(Some(msg))` hard failure; `Err(None)` browser / no shell.
#[allow(clippy::future_not_send)]
async fn ensure_token_from_box(host: &str) -> Result<String, Option<String>> {
    match crate::tauri_bridge::invoke_sync_api_token_from_box(host).await {
        Ok(tok) => Ok(tok),
        Err(e) if e.contains("desktop shell") => Err(None),
        Err(e) => Err(Some(format!(
            "Could not load the Status API token from the box: {e}"
        ))),
    }
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
