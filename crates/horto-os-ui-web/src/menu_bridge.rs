//! Bridge native Tauri menu actions into the Leptos app via `horto-menu` DOM events.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::Screen;

/// Listen for `horto-menu` CustomEvents emitted by the Tauri shell.
pub fn attach_menu_bridge(
    screen: RwSignal<Screen>,
    theme: RwSignal<String>,
    on_refresh: Callback<()>,
    about_open: RwSignal<bool>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        let Some(custom) = event.dyn_ref::<web_sys::CustomEvent>() else {
            return;
        };
        let detail = custom.detail();
        let Ok(action) = js_sys::Reflect::get(&detail, &"action".into()) else {
            return;
        };
        let Some(action) = action.as_string() else {
            return;
        };
        match action.as_str() {
            "refreshStatus" => on_refresh.run(()),
            "goOverview" => screen.set(Screen::Overview),
            "goConnection" => screen.set(Screen::Connection),
            "goServices" => screen.set(Screen::Services),
            "goLogs" => screen.set(Screen::Logs),
            "cycleTheme" => {
                let next = crate::cycle_theme(&theme.get_untracked());
                crate::apply_theme(&next);
                theme.set(next);
            }
            "about" => about_open.set(true),
            _ => {}
        }
    }) as Box<dyn FnMut(_)>);

    let _ = window.add_event_listener_with_callback("horto-menu", closure.as_ref().unchecked_ref());
    closure.forget();
}
