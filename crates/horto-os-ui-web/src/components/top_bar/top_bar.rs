use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::Snapshot;
use crate::Screen;

include!(concat!(env!("OUT_DIR"), "/rangular/top_bar_view.rs"));

#[component]
pub fn TopBarPanel(
    screen: RwSignal<Screen>,
    theme: RwSignal<String>,
    snap: RwSignal<Snapshot>,
    busy: RwSignal<bool>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    top_bar_view(HostCell::new(TopBarHost {
        screen,
        theme,
        snap,
        busy,
        on_refresh,
    }))
}

struct TopBarHost {
    screen: RwSignal<Screen>,
    theme: RwSignal<String>,
    snap: RwSignal<Snapshot>,
    busy: RwSignal<bool>,
    on_refresh: Callback<()>,
}

fn link_state(snap: &Snapshot) -> (&'static str, &'static str) {
    if snap.error.is_some() || snap.health_ok == Some(false) {
        ("down", "Status API unreachable. Click to retry.")
    } else if snap.health_ok == Some(true) {
        ("up", "Connected to Status API. Click to refresh.")
    } else {
        ("pending", "Not connected yet. Click to connect.")
    }
}

impl Host for TopBarHost {
    fn get(&self, name: &str) -> Option<Value> {
        let screen = self.screen.get();
        let theme = self.theme.get();
        let snap = self.snap.get();
        let busy = self.busy.get();
        let (state, title) = link_state(&snap);
        match name {
            "overviewActive" => Some(Value::Bool(screen == Screen::Overview)),
            "connectionActive" => Some(Value::Bool(screen == Screen::Connection)),
            "servicesActive" => Some(Value::Bool(screen == Screen::Services)),
            "logsActive" => Some(Value::Bool(screen == Screen::Logs)),
            "isSystem" => Some(Value::Bool(theme == "system")),
            "isLight" => Some(Value::Bool(theme == "light")),
            "isDark" => Some(Value::Bool(theme == "dark")),
            "themeTitle" => Some(Value::Str(format!(
                "Theme: {theme} (click to cycle system / light / dark)"
            ))),
            "busy" => Some(Value::Bool(busy)),
            "linkUp" => Some(Value::Bool(state == "up")),
            "linkDown" => Some(Value::Bool(state == "down")),
            "linkPending" => Some(Value::Bool(state == "pending")),
            "linkTitle" => Some(Value::Str(title.into())),
            _ => None,
        }
    }

    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, HostError> {
        match name {
            "goOverview" => self.screen.set(Screen::Overview),
            "goConnection" => self.screen.set(Screen::Connection),
            "goServices" => self.screen.set(Screen::Services),
            "goLogs" => self.screen.set(Screen::Logs),
            "cycleTheme" => {
                let next = crate::cycle_theme(&self.theme.get());
                crate::apply_theme(&next);
                self.theme.set(next);
            }
            "retryConnection" if !self.busy.get() => self.on_refresh.run(()),
            _ => {}
        }
        Ok(Value::Unit)
    }
}
