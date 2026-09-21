//! Keyboard handling for the TUI event loop.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use horto_os_ui_shared::SetupKind;
use std::sync::atomic::Ordering;

use crate::app::{App, Modal};
use crate::prompt::{confirm_key, ConfirmResult, TextInputResult};
use crate::tabs::Screen;

pub fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q' | 'Q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub fn ctrl_c_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub fn handle_screen_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('?') => {
            app.help_open = true;
            app.message = "Help".into();
        }
        KeyCode::Tab => {
            app.apply = !app.apply;
            app.message = if app.apply {
                "APPLY".into()
            } else {
                "PLAN".into()
            };
            if app.remote.is_none() {
                app.refresh();
            }
        }
        KeyCode::Char(d) if d.is_ascii_digit() => {
            if let Some(screen) = Screen::from_digit(d, app.is_remote()) {
                app.select_screen(screen);
            }
        }
        KeyCode::Char('c') if app.screen == Screen::Logs => app.clear_logs(),
        KeyCode::Char('r') => {
            app.refresh();
            if app.remote.is_none() {
                app.message = "Refreshed".into();
            }
        }
        KeyCode::Char('f' | 'F') => {
            app.fetch_current_tab();
        }
        KeyCode::Char('a') => app.run_all(),
        KeyCode::Char('b') => app.run_backup_etc(),
        KeyCode::Char('B') => app.show_disk_backup_status(),
        KeyCode::Char('p' | 'P') => {
            let next = if app.kind == SetupKind::Full {
                SetupKind::Minimal
            } else {
                SetupKind::Full
            };
            app.set_pipeline_kind_local(next);
        }
        KeyCode::Char('e' | 'E') if app.screen == Screen::Ssh => {
            app.open_host_editor();
        }
        KeyCode::Char('i' | 'I') if app.screen == Screen::Ssh => {
            app.install_ssh_key_action();
        }
        KeyCode::Enter if app.screen == Screen::Setup => {
            app.run_selected();
        }
        KeyCode::Enter
            if matches!(
                app.screen,
                Screen::Ssh
                    | Screen::Cli
                    | Screen::Api
                    | Screen::Mcp
                    | Screen::Overview
                    | Screen::Reboot
            ) =>
        {
            app.run_surface_enter();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(i) = app.step_state.selected() {
                if i > 0 {
                    app.step_state.select(Some(i - 1));
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.status_lines.len();
            if let Some(i) = app.step_state.selected() {
                if i + 1 < len {
                    app.step_state.select(Some(i + 1));
                }
            }
        }
        KeyCode::Left => {
            app.prev_screen();
        }
        KeyCode::Right => {
            app.next_screen();
        }
        _ => {}
    }
}

/// Handle confirm / text / secret modals. Returns true when the key was consumed.
pub fn handle_modal_key(app: &mut App, key: KeyEvent) -> bool {
    match app.modal {
        Some(Modal::TextHost(_)) => {
            let Some(Modal::TextHost(mut input)) = app.modal.take() else {
                return true;
            };
            match input.handle_key(key) {
                TextInputResult::Continue => {
                    app.modal = Some(Modal::TextHost(input));
                }
                TextInputResult::Submit(value) => {
                    app.apply_host_edit(&value);
                }
                TextInputResult::Cancel => {
                    app.push_log("Host edit cancelled");
                }
            }
            true
        }
        Some(Modal::SudoPassword(_)) => {
            let Some(Modal::SudoPassword(mut input)) = app.modal.take() else {
                return true;
            };
            match input.handle_key(key) {
                TextInputResult::Continue => {
                    app.modal = Some(Modal::SudoPassword(input));
                }
                TextInputResult::Submit(password) => {
                    if password.is_empty() {
                        app.push_log("Reboot cancelled (empty password)");
                        app.note("Reboot cancelled");
                    } else {
                        app.do_reboot(password);
                    }
                }
                TextInputResult::Cancel => {
                    app.push_log("Reboot cancelled");
                    app.note("Reboot cancelled");
                }
            }
            true
        }
        Some(Modal::Rebooting) => {
            if key.code == KeyCode::Esc {
                app.reboot_cancel.store(true, Ordering::SeqCst);
                app.reboot_inflight = false;
                app.reboot_rx = None;
                app.modal = None;
            }
            true
        }
        Some(Modal::Confirm(_)) => {
            match confirm_key(key) {
                ConfirmResult::Yes => app.resolve_confirm_yes(),
                ConfirmResult::No => app.resolve_confirm_no(),
                ConfirmResult::Ignore => {}
            }
            true
        }
        None => false,
    }
}
