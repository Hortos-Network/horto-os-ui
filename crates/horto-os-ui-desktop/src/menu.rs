//! Native application menu (File / Edit / View / Help), shaped like other Tauri 2 desks on this machine.

use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const MENU_ACTION_EVENT: &str = "menu-action";
const MENU_DOM_EVENT: &str = "horto-menu";

fn main_webview(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main").or_else(|| {
        app.webview_windows()
            .into_values()
            .find(|w| w.is_focused().unwrap_or(false))
    })
}

fn emit_action(app: &AppHandle, action: &str) {
    let payload = serde_json::json!({ "action": action });
    let _ = app.emit(MENU_ACTION_EVENT, payload);

    let script = format!(
        r#"window.dispatchEvent(new CustomEvent({event},{{detail:{{action:{action}}}}}));"#,
        event = serde_json::to_string(MENU_DOM_EVENT).unwrap_or_else(|_| "\"\"".into()),
        action = serde_json::to_string(action).unwrap_or_else(|_| "\"\"".into()),
    );
    if let Some(win) = main_webview(app) {
        let _ = win.eval(&script);
    }
}

fn reload_webview(app: &AppHandle) {
    let Some(win) = main_webview(app) else {
        return;
    };
    // Prefer native reload; fall back to location.reload for older webviews.
    if win.eval("window.location.reload(true)").is_err() {
        let _ = win.eval("window.location.reload()");
    }
}

fn handle_menu_id(app: &AppHandle, id: &str) {
    match id {
        "quit" => app.exit(0),
        "reload" | "forceReload" => reload_webview(app),
        "toggleDevtools" =>
        {
            #[cfg(debug_assertions)]
            if let Some(win) = main_webview(app) {
                if win.is_devtools_open() {
                    win.close_devtools();
                } else {
                    win.open_devtools();
                }
            }
        }
        "resetZoom" => {
            if let Some(win) = main_webview(app) {
                let _ = win.eval("document.body.style.zoom='1'");
            }
        }
        "zoomIn" => {
            if let Some(win) = main_webview(app) {
                let _ = win.eval(
                    "document.body.style.zoom=String((parseFloat(document.body.style.zoom||'1')||1)+0.1)",
                );
            }
        }
        "zoomOut" => {
            if let Some(win) = main_webview(app) {
                let _ = win.eval(
                    "document.body.style.zoom=String(Math.max(0.5,(parseFloat(document.body.style.zoom||'1')||1)-0.1))",
                );
            }
        }
        "refreshStatus" | "goOverview" | "goConnection" | "goServices" | "cycleTheme" | "about" => {
            emit_action(app, id)
        }
        _ => {}
    }
}

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let refresh = MenuItemBuilder::with_id("refreshStatus", "Refresh Status")
        .accelerator("CmdOrCtrl+Shift+R")
        .build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;

    let file = SubmenuBuilder::new(app, "File")
        .item(&refresh)
        .separator()
        .item(&quit)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let connection = MenuItemBuilder::with_id("goConnection", "Connection")
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let overview = MenuItemBuilder::with_id("goOverview", "Overview")
        .accelerator("CmdOrCtrl+2")
        .build(app)?;
    let services = MenuItemBuilder::with_id("goServices", "Services")
        .accelerator("CmdOrCtrl+3")
        .build(app)?;
    let theme = MenuItemBuilder::with_id("cycleTheme", "Cycle Theme")
        .accelerator("CmdOrCtrl+T")
        .build(app)?;
    let reload = MenuItemBuilder::with_id("reload", "Reload")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let force = MenuItemBuilder::with_id("forceReload", "Force Reload")
        .accelerator("CmdOrCtrl+Shift+F5")
        .build(app)?;
    let zoom_in = MenuItemBuilder::with_id("zoomIn", "Zoom In")
        .accelerator("CmdOrCtrl+Plus")
        .build(app)?;
    let zoom_out = MenuItemBuilder::with_id("zoomOut", "Zoom Out")
        .accelerator("CmdOrCtrl+-")
        .build(app)?;
    let zoom_reset = MenuItemBuilder::with_id("resetZoom", "Actual Size")
        .accelerator("CmdOrCtrl+0")
        .build(app)?;
    let devtools = MenuItemBuilder::with_id("toggleDevtools", "Toggle Developer Tools")
        .accelerator("CmdOrCtrl+Shift+I")
        .build(app)?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&connection)
        .item(&overview)
        .item(&services)
        .separator()
        .item(&theme)
        .separator()
        .item(&reload)
        .item(&force)
        .separator()
        .item(&zoom_reset)
        .item(&zoom_in)
        .item(&zoom_out)
        .separator()
        .item(&devtools)
        .build()?;

    let about = MenuItemBuilder::with_id("about", "About Horto").build(app)?;
    let help = SubmenuBuilder::new(app, "Help").item(&about).build()?;

    Menu::with_items(app, &[&file, &edit, &view, &help])
}

pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    handle_menu_id(app, event.id().as_ref());
}
