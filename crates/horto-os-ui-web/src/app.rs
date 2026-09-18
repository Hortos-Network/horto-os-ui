use leptos::prelude::*;

use crate::components::{
    BoxStatusPanel, ConnectionPanel, ContainersPanel, ServicesPanel, TopBarPanel,
};
use crate::menu_bridge::attach_menu_bridge;
use crate::status::{fetch_snapshot, Snapshot};
use crate::{apply_theme, build_footer, default_box_url, default_theme, save_box_url, Screen};

#[component]
pub fn App() -> impl IntoView {
    let url = RwSignal::new(default_box_url());
    let token = RwSignal::new(String::new());
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
        save_box_url(&base);
        let tok = {
            let t = token.get();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        };
        busy.set(true);
        leptos::task::spawn_local(async move {
            let next = fetch_snapshot(base, tok).await;
            snap.set(next);
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
            <TopBarPanel screen=screen theme=theme />
            <main class="shell">
                <Show when=move || {
                    matches!(screen.get(), Screen::Connection | Screen::Overview)
                } fallback=|| ()>
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
                        snap.get().status.map(|st| {
                            view! {
                                <BoxStatusPanel status=st.clone() />
                                <ContainersPanel containers=st.containers />
                            }
                        })
                    }}
                </Show>

                <Show when=move || screen.get() == Screen::Services fallback=|| ()>
                    {move || {
                        snap.get().status.map(|st| {
                            view! { <ServicesPanel urls=st.urls /> }
                        })
                    }}
                </Show>

                <p class="footer-note">
                    {move || {
                        format!(
                            "View-only. Install, backup apply, and network stay on the box (CLI or TUI). · {}",
                            build_footer()
                        )
                    }}
                </p>
            </main>
        </div>

        <Show when=move || about_open.get() fallback=|| ()>
            <div class="about-backdrop" on:click=move |_| about_open.set(false)>
                <div class="about-dialog" role="dialog" aria-labelledby="about-title" on:click=move |ev| ev.stop_propagation()>
                    <img class="about-logo" src="/hortos-logo.png" width="56" height="56" alt="" />
                    <h2 id="about-title">"About Horto"</h2>
                    <p>
                        "Homeowner desktop for your Horto box. Status via the box API; setup stays on the box."
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
