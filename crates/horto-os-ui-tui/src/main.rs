//! Horto OS UI TUI (`horto-os-ui-tui`): Ratatui wizard over the shared engine.
//!
//! Tabs: Setup, Overview, SSH, CLI, API, MCP, Reboot, Logs. With `--remote`,
//! probes and installs run over SSH; secrets use askpass, not leave-alt-screen
//! suspend. Confirm and text prompts are Ratatui modals.

mod app;
mod draw;
mod event;
mod probe_job;
mod prompt;
mod tabs;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor::{Hide, Show},
    event::{self as cterm_event, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear as CtClear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, stdout, Write as IoWrite};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

use app::{App, Cli};
use draw::ui;
use event::{ctrl_c_quit, handle_modal_key, handle_screen_key, is_quit};

/// Set by SIGINT/SIGTERM so the loop can restore the tty before exit.
static STOP: AtomicBool = AtomicBool::new(false);

fn install_signal_handlers() {
    let _ = ctrlc::set_handler(|| {
        STOP.store(true, Ordering::SeqCst);
    });
}

fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

/// Restores the terminal on drop (normal exit, `?`, or unwind).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<(Self, Terminal<CrosstermBackend<io::Stdout>>)> {
        enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            Hide,
            CtClear(ClearType::All),
            CtClear(ClearType::Purge)
        )?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;
        Ok((Self, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Reset SGR, show cursor, leave alt buffer, clear leftover TUI cells.
fn hard_reset_tty() {
    let mut out = stdout();
    let _ = write!(
        out,
        "\x1b[0m\x1b[?25h\x1b[?1049l\x1b[?47l\x1b[2J\x1b[3J\x1b[H"
    );
    let _ = out.execute(CtClear(ClearType::All));
    let _ = out.execute(CtClear(ClearType::Purge));
    let _ = out.execute(Show);
    let _ = out.flush();
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = out.execute(LeaveAlternateScreen);
    let _ = out.flush();
    hard_reset_tty();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    install_signal_handlers();
    install_panic_hook();
    let (_guard, mut terminal) = TerminalGuard::enter()?;
    let mut app = App::new(&cli);
    run_app(&mut terminal, &mut app)
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut boot_remote_probe = app.is_remote();
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(());
        }
        app.poll_probe();
        app.poll_reboot();
        app.poll_fetch();
        terminal.draw(|f| ui(f, app))?;
        if boot_remote_probe {
            app.start_remote_probe();
            boot_remote_probe = false;
        }
        if !cterm_event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = cterm_event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if ctrl_c_quit(key) || is_quit(key) {
            return Ok(());
        }
        if handle_modal_key(app, key) {
            continue;
        }
        if app.help_open {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                app.help_open = false;
            }
            continue;
        }
        if key.code == KeyCode::Esc {
            return Ok(());
        }
        handle_screen_key(app, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::BoxCliView;
    use crate::draw::{footer_cli_label, setup_step_line};
    use clap::CommandFactory;
    use clap::Parser;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use horto_os_ui_shared::RemoteBoxCliStatus;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_flags() {
        let cli = Cli::try_parse_from(["horto-os-ui-tui", "--apply", "--minimal", "--skip-piper"])
            .unwrap();
        assert!(cli.apply);
        assert!(cli.minimal);
        assert!(cli.skip_piper);
    }

    #[test]
    fn quit_keys() {
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let plain_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(is_quit(q));
        assert!(is_quit(ctrl_c));
        assert!(!is_quit(plain_c));
    }

    #[test]
    fn footer_cli_label_probing_not_missing() {
        let label = footer_cli_label("0.1.0 (abc)", true, &BoxCliView::Probing);
        assert_eq!(label, "local=0.1.0 (abc) box=probing...");
        assert!(!label.contains('?'));
        assert!(!label.contains("missing"));
    }

    #[test]
    fn footer_cli_label_known_statuses() {
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Missing)
            ),
            "local=0.1.0 box=missing"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::AuthFailed)
            ),
            "local=0.1.0 box=auth failed"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Unreachable)
            ),
            "local=0.1.0 box=unreachable"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Found("0.1.0 (deadbeef)".into()))
            ),
            "local=0.1.0 box=0.1.0 (deadbeef)"
        );
        assert_eq!(
            footer_cli_label("0.1.0", false, &BoxCliView::Probing),
            "local=0.1.0"
        );
    }

    #[test]
    fn remote_app_starts_with_box_probing() {
        let cli = Cli::try_parse_from(["horto-os-ui-tui", "--remote", "horto"]).unwrap();
        let app = App::new(&cli);
        assert_eq!(app.box_cli, BoxCliView::Probing);
        assert!(app.s0_line().contains("| probing |"));
        assert!(!app.cli_current);
    }

    #[test]
    fn setup_step_line_splits_id_status_title() {
        let line = setup_step_line("s0 | pending | Sync CLI to box");
        assert_eq!(line.spans.len(), 5);
        assert_eq!(line.spans[0].content.as_ref(), "s0");
        assert_eq!(line.spans[2].content.as_ref(), "pending");
        assert_eq!(line.spans[4].content.as_ref(), "Sync CLI to box");
    }
}
