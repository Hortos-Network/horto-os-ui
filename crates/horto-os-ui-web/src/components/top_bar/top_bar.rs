use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::Screen;

include!(concat!(env!("OUT_DIR"), "/rangular/top_bar_view.rs"));

#[component]
pub fn TopBarPanel(screen: RwSignal<Screen>, theme: RwSignal<String>) -> impl IntoView {
    top_bar_view(HostCell::new(TopBarHost { screen, theme }))
}

struct TopBarHost {
    screen: RwSignal<Screen>,
    theme: RwSignal<String>,
}

impl Host for TopBarHost {
    fn get(&self, name: &str) -> Option<Value> {
        let screen = self.screen.get();
        let theme = self.theme.get();
        match name {
            "overviewActive" => Some(Value::Bool(screen == Screen::Overview)),
            "connectionActive" => Some(Value::Bool(screen == Screen::Connection)),
            "servicesActive" => Some(Value::Bool(screen == Screen::Services)),
            "isSystem" => Some(Value::Bool(theme == "system")),
            "isLight" => Some(Value::Bool(theme == "light")),
            "isDark" => Some(Value::Bool(theme == "dark")),
            "themeTitle" => Some(Value::Str(format!(
                "Theme: {theme} (click to cycle system / light / dark)"
            ))),
            _ => None,
        }
    }

    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, HostError> {
        match name {
            "goOverview" => self.screen.set(Screen::Overview),
            "goConnection" => self.screen.set(Screen::Connection),
            "goServices" => self.screen.set(Screen::Services),
            "cycleTheme" => {
                let next = crate::cycle_theme(&self.theme.get());
                crate::apply_theme(&next);
                self.theme.set(next);
            }
            _ => {}
        }
        Ok(Value::Unit)
    }
}
