use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear as CtClear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use horto_os_ui_shared::{
    backup_etc_timestamped, box_status, offer_save_api_token, probe_disk_backup, remote_run_cli,
    require_root_for_apply, setup_run, setup_step, ApplyMode, DiskBackupOpts, HostContext,
    RemoteOptions, RemoteRunRequest, SetupKind, StdioPrompts, SystemProcessRunner, GIT_COMMIT,
    LONG_VERSION, VERSION,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::io::{self, stdout, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

const READY: &str = "Ready (? help)";

/// Set by SIGINT/SIGTERM so the loop can restore the tty before exit.
static STOP: AtomicBool = AtomicBool::new(false);

fn footer_hints(app: &App) -> &'static str {
    if app.help_open {
        return "Esc or ? close help";
    }
    if app.confirm_destructive.is_some() {
        return "Enter/y confirm · Esc/n cancel · Ctrl+C quit";
    }
    match app.screen {
        Screen::Setup => "j/k select · Enter run · a all · b backup · d dry-run · ? help · q quit",
        Screen::Logs => "Tab/1-3 screens · r refresh · B disk probe · ? help · q quit",
        Screen::Overview => "Tab/1-3 screens · Left/Right kind · r refresh · ? help · q quit",
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-tui",
    about = "Horto OS UI terminal",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    minimal: bool,
    #[arg(long)]
    skip_piper: bool,
    /// OpenSSH Host alias or user@host; run setup via remote runner
    #[arg(long)]
    remote: Option<String>,
    /// Opt-in: install this PC's public key on the box. Off by default.
    #[arg(long, default_value_t = false)]
    install_ssh_key: bool,
    /// Local directory with box binaries (skips GitHub Release download)
    #[arg(long, env = "HORTO_BIN_DIR")]
    bin_dir: Option<std::path::PathBuf>,
    /// GitHub Release tag for box tar.gz (`v0.1.0` or tip `dev-preview`)
    #[arg(long, env = "HORTO_RELEASE_TAG")]
    release_tag: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup,
    Logs,
    Overview,
}

struct App {
    screen: Screen,
    dry_run: bool,
    kind: SetupKind,
    skip_piper: bool,
    remote: Option<String>,
    install_ssh_key: bool,
    bin_dir: Option<std::path::PathBuf>,
    release_tag: Option<String>,
    step_state: ListState,
    logs: Vec<String>,
    status_lines: Vec<String>,
    overview_text: String,
    confirm_destructive: Option<String>,
    help_open: bool,
    message: String,
}

impl App {
    fn new(cli: &Cli) -> Self {
        let kind = if cli.minimal {
            SetupKind::Minimal
        } else {
            SetupKind::Full
        };
        let mut step_state = ListState::default();
        step_state.select(Some(0));
        let mut app = Self {
            screen: Screen::Setup,
            dry_run: cli.dry_run,
            kind,
            skip_piper: cli.skip_piper,
            remote: cli.remote.clone(),
            install_ssh_key: cli.install_ssh_key,
            bin_dir: cli.bin_dir.clone(),
            release_tag: cli.release_tag.clone(),
            step_state,
            logs: Vec::new(),
            status_lines: Vec::new(),
            overview_text: String::new(),
            confirm_destructive: None,
            help_open: false,
            message: READY.into(),
        };
        app.refresh();
        app
    }

    fn remote_opts(&self) -> Option<RemoteOptions> {
        self.remote.as_ref().map(|host| {
            let mut opts = RemoteOptions {
                host: host.clone(),
                install_ssh_key: self.install_ssh_key,
                bin_dir: self.bin_dir.clone(),
                ..RemoteOptions::default()
            };
            if let Some(tag) = self
                .release_tag
                .as_ref()
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
            {
                opts.release_tag = tag.to_owned();
            }
            opts
        })
    }

    fn run_remote_cli(&mut self, rest: &[&str], use_sudo: bool, install_payload: bool) {
        let Some(options) = self.remote_opts() else {
            return;
        };
        let mut cli_args = Vec::new();
        if self.dry_run {
            cli_args.push("--dry-run".into());
        }
        if self.skip_piper {
            cli_args.push("--skip-piper".into());
        }
        for a in rest {
            cli_args.push((*a).to_owned());
        }
        match remote_run_cli(
            &SystemProcessRunner,
            &RemoteRunRequest {
                options,
                cli_args,
                use_sudo,
                install_payload_on_success: install_payload,
                offer_reboot_on_success: install_payload && !self.dry_run,
            },
        ) {
            Ok(outcome) => {
                for line in outcome.log.lines() {
                    self.push_log(line.to_owned());
                }
                if let Some(token) = outcome.api_token.as_deref() {
                    match offer_save_api_token(token) {
                        Ok(true) => {
                            self.push_log(
                                "Saved status-api bearer to ~/.config/horto-os-ui/api_token"
                                    .to_owned(),
                            );
                            self.message = "Remote finished; API token saved".into();
                        }
                        Ok(false) => {
                            self.push_log("Skipped saving status-api bearer locally");
                            self.message = "Remote command finished".into();
                        }
                        Err(e) => {
                            self.push_log(format!("Token save failed: {e}"));
                            self.message = "Remote finished; token save failed".into();
                        }
                    }
                } else {
                    self.message = "Remote command finished".into();
                }
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Remote failed: {e}");
            }
        }
    }

    fn make_ctx(&self) -> HostContext {
        let mode = if self.dry_run {
            ApplyMode::DryRun
        } else {
            ApplyMode::Apply
        };
        let mut ctx = HostContext::new(mode, self.kind).with_prompts(Box::new(StdioPrompts));
        ctx.skip_piper = self.skip_piper;
        ctx
    }

    fn refresh(&mut self) {
        let ctx = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx, self.kind);
        self.status_lines = report
            .steps
            .iter()
            .map(|s| {
                format!(
                    "{} | {} | {}{}",
                    s.id,
                    s.status,
                    s.title,
                    if s.destructive { " *" } else { "" }
                )
            })
            .collect();
        let box_st = box_status(&ctx, self.kind);
        let mut overview = String::new();
        if let Some(host) = &self.remote {
            overview.push_str(&format!(
                "Mode: remote ({host})  install_ssh_key={}\n",
                self.install_ssh_key
            ));
        } else {
            overview.push_str("Mode: embedded\n");
        }
        overview.push_str(&format!("Hostname: {}\n", box_st.hostname));
        overview.push_str(&format!(
            "Root: {}  Docker: {}  Full env: {}  Minimal env: {}\n",
            box_st.doctor.is_root,
            box_st.doctor.docker_present,
            box_st.doctor.full_env,
            box_st.doctor.minimal_env
        ));
        for n in &box_st.doctor.notes {
            overview.push_str(&format!("- {n}\n"));
        }
        overview.push_str("\nContainers:\n");
        if box_st.containers.is_empty() {
            overview.push_str("  (none)\n");
        } else {
            for c in &box_st.containers {
                overview.push_str(&format!("  {} {}\n", c.names, c.status));
            }
        }
        overview.push_str("\nURLs:\n");
        for u in &box_st.urls {
            let mark = if u.up { "up" } else { "down" };
            overview.push_str(&format!("  {} [{}]: {}\n", u.name, mark, u.url));
        }
        overview.push_str(&format!("\nLeases: {}\n", box_st.leases.len()));
        for l in box_st.leases.iter().take(12) {
            overview.push_str(&format!("  {} {}\n", l.hostname, l.ip));
        }
        overview.push_str("\nBackup:\n");
        overview.push_str(&format!(
            "  initial_setup: {}\n",
            box_st.backup.initial_setup_present
        ));
        if box_st.backup.timestamped.is_empty() {
            overview.push_str("  timestamped: (none)\n");
        } else {
            let recent: Vec<_> = box_st.backup.timestamped.iter().rev().take(5).collect();
            overview.push_str(&format!(
                "  timestamped ({}): {}\n",
                box_st.backup.timestamped.len(),
                recent
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        overview.push_str(&format!(
            "  disk root={} safe={} blockers={}\n",
            box_st.backup.disk.root_source,
            box_st.backup.disk.safe_to_apply,
            box_st.backup.disk.blockers.len()
        ));
        self.overview_text = overview;
    }

    fn selected_step_id(&self) -> Option<String> {
        let idx = self.step_state.selected()?;
        self.status_lines
            .get(idx)
            .and_then(|l| l.split(" | ").next())
            .map(str::to_string)
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.logs.push(line.into());
        if self.logs.len() > 500 {
            self.logs.drain(0..self.logs.len() - 500);
        }
    }

    fn run_selected(&mut self) {
        let Some(id) = self.selected_step_id() else {
            return;
        };
        let ctx_probe = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx_probe, self.kind);
        if let Some(row) = report.steps.iter().find(|s| s.id == id) {
            if row.destructive && !self.dry_run && self.confirm_destructive.is_none() {
                self.confirm_destructive = Some(id.clone());
                self.message = format!("Step {id} is destructive. Enter/y confirm, Esc/n cancel.");
                return;
            }
        }
        self.confirm_destructive = None;
        self.execute_step(&id);
    }

    fn execute_step(&mut self, id: &str) {
        self.push_log(format!("Running step {id} (dry_run={})", self.dry_run));
        if self.remote.is_some() {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(&["setup", "step", id, kind], !self.dry_run, false);
            self.refresh();
            return;
        }
        let mut ctx = self.make_ctx();
        match setup_step(&mut ctx, self.kind, id) {
            Ok(()) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = format!("Step {id} finished");
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Step {id} failed: {e}");
            }
        }
        self.refresh();
    }

    fn run_all(&mut self) {
        self.push_log(format!("Running full pipeline (dry_run={})", self.dry_run));
        if self.remote.is_some() {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(&["setup", "run", kind], !self.dry_run, !self.dry_run);
            self.refresh();
            return;
        }
        let mut ctx = self.make_ctx();
        match setup_run(&mut ctx, self.kind) {
            Ok(()) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = "Pipeline finished".into();
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Pipeline failed: {e}");
            }
        }
        self.refresh();
    }

    fn run_backup_etc(&mut self) {
        self.push_log(format!(
            "Timestamped /etc backup (dry_run={})",
            self.dry_run
        ));
        let mut ctx = self.make_ctx();
        if let Err(e) = require_root_for_apply(ctx.mode) {
            self.push_log(format!("ERROR: {e}"));
            self.message = format!("Backup etc failed: {e}");
            return;
        }
        match backup_etc_timestamped(&mut ctx) {
            Ok(report) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = format!("Backup etc -> {}", report.dest);
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Backup etc failed: {e}");
            }
        }
        self.refresh();
    }

    fn show_disk_backup_status(&mut self) {
        let probe = probe_disk_backup(&DiskBackupOpts::default());
        self.push_log(format!(
            "disk backup: root={} safe={} partclone={}",
            probe.root_source, probe.safe_to_apply, probe.partclone_present
        ));
        for b in &probe.blockers {
            self.push_log(format!("blocker: {b}"));
        }
        self.message = if probe.safe_to_apply {
            "Disk backup looks safe (CLI: horto backup disk)".into()
        } else {
            format!(
                "Disk backup blocked ({}). See Logs. Boot from SD for eMMC image.",
                probe.blockers.len()
            )
        };
        self.screen = Screen::Logs;
    }

    fn next_screen(&mut self) {
        self.screen = match self.screen {
            Screen::Setup => Screen::Logs,
            Screen::Logs => Screen::Overview,
            Screen::Overview => Screen::Setup,
        };
    }

    fn prev_screen(&mut self) {
        self.screen = match self.screen {
            Screen::Setup => Screen::Overview,
            Screen::Logs => Screen::Setup,
            Screen::Overview => Screen::Logs,
        };
    }

    fn select_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.help_open = false;
    }
}

fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

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

/// Leave the ratatui alt screen so SSH/sudo/prompts own the real TTY, then restore.
fn with_suspended_tui<R>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    f: impl FnOnce() -> R,
) -> io::Result<R> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, Show)?;
    let _ = stdout().flush();
    let out = f();
    enable_raw_mode()?;
    execute!(
        stdout(),
        EnterAlternateScreen,
        Hide,
        CtClear(ClearType::All),
        CtClear(ClearType::Purge)
    )?;
    terminal.clear()?;
    Ok(out)
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
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(());
        }
        terminal.draw(|f| ui(f, app))?;
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if is_quit(key) {
            return Ok(());
        }
        if app.help_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    app.help_open = false;
                    app.message = READY.into();
                }
                _ => {}
            }
            continue;
        }
        if app.confirm_destructive.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    with_suspended_tui(terminal, || app.run_selected())?;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    app.confirm_destructive = None;
                    app.message = "Cancelled".into();
                }
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(()),
            KeyCode::Char('?') => {
                app.help_open = true;
                app.message = "Help".into();
            }
            KeyCode::Tab => app.next_screen(),
            KeyCode::BackTab => app.prev_screen(),
            KeyCode::Char('1') => app.select_screen(Screen::Setup),
            KeyCode::Char('2') => app.select_screen(Screen::Logs),
            KeyCode::Char('3') => app.select_screen(Screen::Overview),
            KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.dry_run = !app.dry_run;
                app.message = format!("dry-run = {}", app.dry_run);
                app.refresh();
            }
            KeyCode::Char('r') => {
                app.refresh();
                app.message = "Refreshed".into();
            }
            KeyCode::Char('a') => {
                with_suspended_tui(terminal, || app.run_all())?;
            }
            KeyCode::Char('b') => {
                with_suspended_tui(terminal, || app.run_backup_etc())?;
            }
            KeyCode::Char('B') => app.show_disk_backup_status(),
            KeyCode::Enter if app.screen == Screen::Setup => {
                // Destructive apply: first Enter only arms the confirm dialog (stay in TUI).
                let ask_confirm = !app.dry_run
                    && app
                        .step_state
                        .selected()
                        .and_then(|i| app.status_lines.get(i))
                        .is_some_and(|line| line.contains(" *"));
                if ask_confirm {
                    app.run_selected();
                } else {
                    with_suspended_tui(terminal, || app.run_selected())?;
                }
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
                app.kind = SetupKind::Full;
                app.refresh();
                app.message = "kind = full".into();
            }
            KeyCode::Right => {
                app.kind = SetupKind::Minimal;
                app.refresh();
                app.message = "kind = minimal".into();
            }
            _ => {}
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(f.area());

    let titles = ["1 Setup", "2 Logs", "3 Overview"]
        .iter()
        .map(|t| Line::from(*t))
        .collect::<Vec<_>>();
    let idx = match app.screen {
        Screen::Setup => 0,
        Screen::Logs => 1,
        Screen::Overview => 2,
    };
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(
            Block::default()
                .borders(Borders::ALL)
                // No spaces: some terminals paint border glyphs through title gaps.
                .title(format!("horto-tui/{VERSION}/{GIT_COMMIT}")),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    match app.screen {
        Screen::Setup => draw_setup(f, app, chunks[1]),
        Screen::Logs => draw_logs(f, app, chunks[1]),
        Screen::Overview => draw_overview(f, app, chunks[1]),
    }

    let mode = if app.dry_run { "DRY-RUN" } else { "APPLY" };
    let status = if app.message.is_empty() {
        READY.to_string()
    } else {
        app.message.clone()
    };
    let footer = Paragraph::new(vec![
        Line::from(format!("[{mode}] {} · {}", app.kind.as_str(), status)),
        Line::from(Span::styled(
            footer_hints(app),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(footer, chunks[2]);

    if app.help_open {
        draw_help(f);
    }
}

fn draw_help(f: &mut Frame) {
    let area = centered_rect(70, 70, f.area());
    f.render_widget(Clear, area);
    let body = [
        "Horto TUI help",
        "",
        "q / Esc / Ctrl+C   Quit (terminal restored)",
        "?                  Toggle this help",
        "Tab / Shift-Tab    Next / previous screen",
        "1 2 3              Setup / Logs / Overview",
        "j k / arrows       Move step selection",
        "Left / Right       Full / Minimal kind",
        "Enter              Run selected step",
        "a                  Run full pipeline",
        "b / B              Timestamped /etc backup / disk probe",
        "r                  Refresh status",
        "d                  Toggle dry-run",
        "y / n              Confirm / cancel destructive step",
        "",
        "Mouse capture is off so you can select and copy text.",
        "Press Esc or ? to close.",
    ]
    .join("\n");
    let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

fn draw_setup(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .status_lines
        .iter()
        .map(|l| {
            let style = if l.contains("| done |") {
                Style::default().fg(Color::Green)
            } else if l.contains("| stale |") {
                Style::default().fg(Color::Magenta)
            } else if l.contains("| failed |") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(l.clone(), style)))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Steps (Enter run · a = all · * = destructive)"),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, area, &mut app.step_state);
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let text = if app.logs.is_empty() {
        "(no logs yet - select text with the mouse to copy)".to_string()
    } else {
        app.logs
            .iter()
            .rev()
            .take(40)
            .cloned()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    };
    let p = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(p, area);
}

fn draw_overview(f: &mut Frame, app: &App, area: Rect) {
    let p = Paragraph::new(app.overview_text.clone())
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Overview"));
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_flags() {
        let cli =
            Cli::try_parse_from(["horto-os-ui-tui", "--dry-run", "--minimal", "--skip-piper"])
                .unwrap();
        assert!(cli.dry_run);
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
}
