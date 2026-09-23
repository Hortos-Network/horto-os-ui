//! Thin Tauri 2 shell: native menu + Trunk-built Horto Leptos SPA + remote OpenSSH setup.

mod menu;
mod remote;

use horto_os_ui_shared::LogBus;
use tauri::{Emitter, Manager};

const LOG_DOM_EVENT: &str = "horto-log";

/// Start the desktop webview and block until the process exits.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bus = LogBus::with_default_capacity();
    horto_os_ui_shared::init_tracing_with_bus("info", &bus);

    let bus_for_manage = bus.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(bus_for_manage)
        .invoke_handler(tauri::generate_handler![
            remote::read_api_token,
            remote::write_api_token,
            remote::sync_api_token_from_box,
            remote::local_host_metrics,
            remote::local_containers,
            remote::list_app_logs,
            remote::clear_app_logs,
            remote::append_app_log,
            remote::list_known_remote_hosts_cmd,
            remote::list_release_tags_cmd,
            remote::tip_cli_version,
            remote::remote_probe,
            remote::remote_surfaces_probe,
            remote::remote_surfaces_text,
            remote::remote_upload_cli_cmd,
            remote::remote_setup
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            bus.set_emitter(move |entry| {
                let _ = handle.emit("horto-log", &entry);
                if let Some(win) = handle.get_webview_window("main") {
                    // Double-encode so webview eval cannot break on quotes/newlines in message.
                    let Ok(json) = serde_json::to_string(&entry) else {
                        return;
                    };
                    let Ok(json_lit) = serde_json::to_string(&json) else {
                        return;
                    };
                    let script = format!(
                        r#"window.dispatchEvent(new CustomEvent({event},{{detail:JSON.parse({json_lit})}}));"#,
                        event =
                            serde_json::to_string(LOG_DOM_EVENT).unwrap_or_else(|_| "\"\"".into()),
                        json_lit = json_lit,
                    );
                    let _ = win.eval(&script);
                }
            });

            let menu = menu::build_menu(app.handle())?;
            app.set_menu(menu)?;
            app.on_menu_event(menu::on_menu_event);

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_title(&format!(
                    "Horto v{} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("GIT_COMMIT_HASH")
                ));
                let app_handle = app.handle().clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        app_handle.exit(0);
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running horto-os-ui-desktop");
}
