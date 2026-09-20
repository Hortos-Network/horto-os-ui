use leptos::prelude::*;

use crate::components::{
    BoxStatusPanel, ConnectionPanel, ContainersPanel, ServicesPanel, TopBarPanel,
};
use crate::menu_bridge::attach_menu_bridge;
use crate::status::{fetch_snapshot, Snapshot};
use crate::{
    align_status_api_url_to_hostname, apply_theme, build_footer, default_api_token,
    default_status_api_url, default_theme, save_api_token, save_status_api_url, Screen,
};

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

    Effect::new(move |_| {
        apply_theme(&theme.get());
    });

    let do_refresh = Callback::new(move |()| {
        let base = url.get();
        save_status_api_url(&base);
        let tok = {
            let t = token.get();
            save_api_token(&t);
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        };
        busy.set(true);
        leptos::task::spawn_local(async move {
            let started = js_sys::Date::now();
            let next = fetch_snapshot(base.clone(), tok.clone()).await;
            if let Some(st) = next.status.as_ref() {
                if let Some(aligned) = align_status_api_url_to_hostname(&base, &st.hostname) {
                    // Prefer the box hostname when the loopback URL worked; keep
                    // loopback if the hostname URL does not answer.
                    let aligned_snap = fetch_snapshot(aligned.clone(), tok).await;
                    if aligned_snap.error.is_none() {
                        url.set(aligned.clone());
                        save_status_api_url(&aligned);
                        snap.set(aligned_snap);
                    } else {
                        snap.set(next);
                    }
                } else {
                    snap.set(next);
                }
            } else {
                snap.set(next);
            }
            let elapsed = js_sys::Date::now() - started;
            const MIN_BUSY_MS: f64 = 550.0;
            if elapsed < MIN_BUSY_MS {
                gloo_timers::future::TimeoutFuture::new((MIN_BUSY_MS - elapsed) as u32).await;
            }
            busy.set(false);
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
            do_refresh.run(());
        }
    });

    view! {
        <div class="app">
            <TopBarPanel
                screen=screen
                theme=theme
                snap=snap
                busy=busy
                on_refresh=do_refresh
            />
            <main class="shell">
                <Show when=move || screen.get() == Screen::Connection fallback=|| ()>
                    <ConnectionPanel
                        url=url
                        token=token
                        busy=busy
                        snap=snap
                        on_refresh=do_refresh
                    />
                </Show>

                <Show when=move || screen.get() == Screen::Overview fallback=|| ()>
                    {move || {
                        let api = url.get();
                        snap.get().status.map(|st| {
                            let api_containers = api.clone();
                            view! {
                                <BoxStatusPanel
                                    status=st.clone()
                                    api_base=api
                                    token=token
                                    on_refresh=do_refresh
                                />
                                <ContainersPanel
                                    containers=st.containers
                                    urls=st.urls
                                    api_base=api_containers
                                />
                            }
                        })
                    }}
                </Show>

                <Show when=move || screen.get() == Screen::Services fallback=|| ()>
                    {move || {
                        let api = url.get();
                        snap.get().status.map(|st| {
                            view! { <ServicesPanel urls=st.urls api_base=api /> }
                        })
                    }}
                </Show>

                <p class="footer-note">
                    {move || build_footer()}
                </p>
            </main>
        </div>

        <Show when=move || about_open.get() fallback=|| ()>
            <div class="about-backdrop" on:click=move |_| about_open.set(false)>
                <div class="about-dialog" role="dialog" aria-labelledby="about-title" on:click=move |ev| ev.stop_propagation()>
                    <img class="about-logo" src="/hortos-logo.png" width="56" height="56" alt="" />
                    <h2 id="about-title">"About Horto"</h2>
                    <p>
                        "Homeowner desktop for your Horto box."
                    </p>
                    <p class="about-meta">{move || {
                        format!("horto-os-ui · Tauri 2 · Leptos · {}", build_footer())
                    }}</p>
                    <button type="button" class="about-close" on:click=move |_| about_open.set(false)>
                        "Close"
                    </button>
                </div>
            </div>
        </Show>
    }
}
