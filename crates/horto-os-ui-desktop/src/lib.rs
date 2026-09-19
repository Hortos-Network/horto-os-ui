//! Thin Tauri 2 shell: native menu + Trunk-built Horto Leptos SPA + remote OpenSSH setup.

mod menu;
mod remote;

use tauri::Manager;

/// Start the desktop webview and block until the process exits.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            remote::remote_probe,
            remote::remote_setup
        ])
        .setup(|app| {
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
