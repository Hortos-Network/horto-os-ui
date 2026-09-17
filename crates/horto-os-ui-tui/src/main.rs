use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use horto_os_ui_shared::{
    backup_etc_timestamped, box_status, footer_line, probe_disk_backup, require_root_for_apply,
    setup_run, setup_step, ApplyMode, DiskBackupOpts, HostContext, SetupKind, StdioPrompts,
    LONG_VERSION,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout};

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup,
    Logs,
    Dashboard,
}

struct App {
    screen: Screen,
    dry_run: bool,
    kind: SetupKind,
    skip_piper: bool,
    step_state: ListState,
    logs: Vec<String>,
    status_lines: Vec<String>,
    dash_text: String,
    confirm_destructive: Option<String>,
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
            step_state,
            logs: Vec::new(),
            status_lines: Vec::new(),
            dash_text: String::new(),
            confirm_destructive: None,
            message: "Tab | Enter step | a all | b backup etc | B disk status | r refresh | d dry-run | q".into(),
        };
        app.refresh();
        app
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
        let mut dash = String::new();
        dash.push_str(&format!("Hostname: {}\n", box_st.hostname));
        dash.push_str(&format!(
            "Root: {}  Docker: {}  Full env: {}  Minimal env: {}\n",
            box_st.doctor.is_root,
            box_st.doctor.docker_present,
            box_st.doctor.full_env,
            box_st.doctor.minimal_env
        ));
        for n in &box_st.doctor.notes {
            dash.push_str(&format!("- {n}\n"));
        }
        dash.push_str("\nContainers:\n");
        if box_st.containers.is_empty() {
            dash.push_str("  (none)\n");
        } else {
            for c in &box_st.containers {
                dash.push_str(&format!("  {} {}\n", c.names, c.status));
            }
        }
        dash.push_str("\nURLs:\n");
        for u in &box_st.urls {
            dash.push_str(&format!("  {}: {}\n", u.name, u.url));
        }
        dash.push_str(&format!("\nLeases: {}\n", box_st.leases.len()));
        for l in box_st.leases.iter().take(12) {
            dash.push_str(&format!("  {} {}\n", l.hostname, l.ip));
        }
        dash.push_str("\nBackup:\n");
        dash.push_str(&format!(
            "  initial_setup: {}\n",
            box_st.backup.initial_setup_present
        ));
        if box_st.backup.timestamped.is_empty() {
            dash.push_str("  timestamped: (none)\n");
        } else {
            let recent: Vec<_> = box_st.backup.timestamped.iter().rev().take(5).collect();
            dash.push_str(&format!(
                "  timestamped ({}): {}\n",
                box_st.backup.timestamped.len(),
                recent
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        dash.push_str(&format!(
            "  disk root={} safe={} blockers={}\n",
            box_st.backup.disk.root_source,
            box_st.backup.disk.safe_to_apply,
            box_st.backup.disk.blockers.len()
        ));
        self.dash_text = dash;
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
                self.message = format!(
                    "Step {id} is destructive. Press Enter again to confirm, Esc to cancel."
                );
                return;
            }
        }
        self.confirm_destructive = None;
        self.execute_step(&id);
    }

    fn execute_step(&mut self, id: &str) {
        self.push_log(format!("Running step {id} (dry_run={})", self.dry_run));
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
                self.message = format!("Backup etc → {}", report.dest);
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(&cli);
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    res
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if app.confirm_destructive.is_some() {
                    match key.code {
                        KeyCode::Enter => app.run_selected(),
                        KeyCode::Esc => {
                            app.confirm_destructive = None;
                            app.message = "Cancelled".into();
                        }
                        KeyCode::Char('q') => return Ok(()),
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab => {
                        app.screen = match app.screen {
                            Screen::Setup => Screen::Logs,
                            Screen::Logs => Screen::Dashboard,
                            Screen::Dashboard => Screen::Setup,
                        };
                    }
                    KeyCode::Char('d') => {
                        app.dry_run = !app.dry_run;
                        app.message = format!("dry-run = {}", app.dry_run);
                        app.refresh();
                    }
                    KeyCode::Char('r') => {
                        app.refresh();
                        app.message = "Refreshed".into();
                    }
                    KeyCode::Char('a') => app.run_all(),
                    KeyCode::Char('b') => app.run_backup_etc(),
                    KeyCode::Char('B') => app.show_disk_backup_status(),
                    KeyCode::Enter => {
                        if app.screen == Screen::Setup {
                            app.run_selected();
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
                    KeyCode::Left => app.kind = SetupKind::Full,
                    KeyCode::Right => app.kind = SetupKind::Minimal,
                    _ => {}
                }
                if matches!(key.code, KeyCode::Left | KeyCode::Right) {
                    app.refresh();
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(f.area());

    let titles = ["Setup", "Logs", "Dashboard"]
        .iter()
        .map(|t| Line::from(*t))
        .collect::<Vec<_>>();
    let idx = match app.screen {
        Screen::Setup => 0,
        Screen::Logs => 1,
        Screen::Dashboard => 2,
    };
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(Block::default().borders(Borders::ALL).title("Horto TUI"))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    match app.screen {
        Screen::Setup => draw_setup(f, app, chunks[1]),
        Screen::Logs => draw_logs(f, app, chunks[1]),
        Screen::Dashboard => draw_dash(f, app, chunks[1]),
    }

    let mode = if app.dry_run { "DRY-RUN" } else { "APPLY" };
    let footer = Paragraph::new(format!(
        "[{mode}] kind={} | {} | {}",
        app.kind.as_str(),
        app.message,
        footer_line()
    ))
    .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(footer, chunks[2]);
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
                .title("Steps (Enter run, a = all)"),
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
        "(no logs yet)".to_string()
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

fn draw_dash(f: &mut Frame, app: &App, area: Rect) {
    let p = Paragraph::new(app.dash_text.clone())
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Dashboard"));
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
}
