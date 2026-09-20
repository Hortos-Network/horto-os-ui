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
    backup_etc_timestamped, box_status, finish_save_api_token, pipeline, probe_disk_backup,
    probe_surfaces, remote_run_cli, remote_upload_cli, require_root_for_apply, setup_run,
    setup_step, ApplyMode, DiskBackupOpts, HostContext, RemoteBoxCliStatus, RemoteOptions,
    RemoteRunRequest, SetupKind, StdioPrompts, SurfaceProbeReport, SystemProcessRunner, GIT_COMMIT,
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
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

mod probe_job;
mod prompt;
mod tabs;
use probe_job::{run_remote_probe, RemoteProbeOk, RemoteProbeOutcome};
use prompt::{
    confirm_key, draw_confirm, draw_rebooting, draw_secret_input, draw_text_input, ConfirmResult,
    SecretInput, TextInput, TextInputResult,
};
use tabs::Screen;

/// In-TUI overlay (confirm / free-text / sudo password).
#[derive(Debug, Clone)]
enum Modal {
    Confirm(ConfirmKind),
    TextHost(TextInput),
    SudoPassword(SecretInput),
    /// Wait while reboot SSH runs on a background thread.
    Rebooting,
}

#[derive(Debug, Clone)]
enum ConfirmKind {
    DestructiveStep(String),
    Reboot,
    SaveToken(String),
    RebootAfterApply,
}

impl ConfirmKind {
    fn title(&self) -> &'static str {
        match self {
            Self::DestructiveStep(_) => "Confirm destructive step",
            Self::Reboot => "Confirm reboot",
            Self::SaveToken(_) => "Save API token",
            Self::RebootAfterApply => "Reboot after apply",
        }
    }

    fn body(&self) -> String {
        match self {
            Self::DestructiveStep(id) => {
                format!("Step {id} is destructive. Continue?")
            }
            Self::Reboot => "Reboot the box now?".into(),
            Self::SaveToken(_) => {
                "Save status-api bearer to ~/.config/horto-os-ui/api_token?".into()
            }
            Self::RebootAfterApply => "Remote apply succeeded. Reboot the box now?".into(),
        }
    }
}

/// Set by SIGINT/SIGTERM so the loop can restore the tty before exit.
static STOP: AtomicBool = AtomicBool::new(false);

/// Box CLI footer/overview value (remote mode).
#[derive(Debug, Clone, PartialEq, Eq)]
enum BoxCliView {
    /// Probe in flight or not yet applied (never shown as missing).
    Probing,
    /// Last probe result from shared remote layer.
    Known(RemoteBoxCliStatus),
}

impl BoxCliView {
    fn as_label(&self) -> &str {
        match self {
            Self::Probing => "probing...",
            Self::Known(s) => s.as_label(),
        }
    }
}

#[cfg(test)]
fn footer_cli_label(cli_local: &str, remote: bool, box_cli: &BoxCliView) -> String {
    if remote {
        format!("local={cli_local} box={}", box_cli.as_label())
    } else {
        format!("local={cli_local}")
    }
}

/// Session state + last user message on the footer status line.
fn footer_status_line(app: &App) -> Line<'static> {
    let mode = if app.dry_run { "DRY-RUN" } else { "APPLY" };
    let mode_style = if app.dry_run {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    };
    let value_style = Style::default().fg(Color::Cyan);
    let mut spans = vec![
        Span::styled(format!("[{mode}]"), mode_style),
        Span::raw(" pipeline="),
        Span::styled(app.kind.as_str().to_owned(), value_style),
        Span::raw(" · local="),
        Span::styled(app.cli_local.clone(), value_style),
    ];
    if app.is_remote() {
        spans.push(Span::raw(" box="));
        spans.push(Span::styled(app.box_cli.as_label().to_owned(), value_style));
    }
    if !app.message.is_empty() {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            app.message.clone(),
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

fn footer_key(label: &str) -> Span<'static> {
    Span::styled(
        label.to_owned(),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_muted(text: &str) -> Span<'static> {
    Span::styled(text.to_owned(), Style::default().fg(Color::DarkGray))
}

/// Panel / tab content title (matches selected-tab orange).
fn panel_title(name: &str) -> Span<'static> {
    Span::styled(
        name.to_owned(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_hints_line(app: &App) -> Line<'static> {
    if app.help_open {
        return Line::from(vec![
            footer_muted("Esc or "),
            footer_key("?"),
            footer_muted(" close help"),
        ]);
    }
    match &app.modal {
        Some(Modal::Confirm(_)) => {
            return Line::from(vec![
                footer_key("Enter"),
                footer_muted("/"),
                footer_key("y"),
                footer_muted(" confirm · "),
                footer_key("Esc"),
                footer_muted("/"),
                footer_key("n"),
                footer_muted(" cancel · "),
                footer_key("Ctrl+C"),
                footer_muted(" quit"),
            ]);
        }
        Some(Modal::TextHost(_)) => {
            return Line::from(vec![
                footer_muted("Type host · "),
                footer_key("Enter"),
                footer_muted(" submit · "),
                footer_key("Esc"),
                footer_muted(" cancel · "),
                footer_key("Ctrl+C"),
                footer_muted(" quit"),
            ]);
        }
        Some(Modal::SudoPassword(_)) => {
            return Line::from(vec![
                footer_muted("Sudo password · "),
                footer_key("Enter"),
                footer_muted(" submit · "),
                footer_key("Esc"),
                footer_muted(" cancel · "),
                footer_key("Ctrl+C"),
                footer_muted(" quit"),
            ]);
        }
        Some(Modal::Rebooting) => {
            return Line::from(vec![
                footer_muted("Rebooting… · "),
                footer_key("Ctrl+C"),
                footer_muted(" quit"),
            ]);
        }
        None => {}
    }
    match app.screen {
        Screen::Setup => Line::from(vec![
            footer_key("j/k"),
            footer_muted("/"),
            footer_key("↑/↓"),
            footer_muted(" select · "),
            footer_key("←/→"),
            footer_muted(" tabs · "),
            footer_key("Enter"),
            footer_muted(" run · "),
            footer_key("a"),
            footer_muted(" all · "),
            footer_key("b"),
            footer_muted(" backup · "),
            footer_key("p"),
            footer_muted(" pipeline · "),
            footer_key("Tab"),
            footer_muted(" dry-run/apply · "),
            footer_key("r"),
            footer_muted(" refresh · "),
            footer_key("?"),
            footer_muted(" help · "),
            footer_key("q"),
            footer_muted(" quit"),
        ]),
        Screen::Logs => Line::from(vec![
            footer_key("c"),
            footer_muted(" clear · "),
            footer_key("←/→"),
            footer_muted(" tabs · "),
            footer_key("Tab"),
            footer_muted(" dry-run/apply · "),
            footer_key("r"),
            footer_muted(" refresh · "),
            footer_key("B"),
            footer_muted(" disk · "),
            footer_key("?"),
            footer_muted(" help · "),
            footer_key("q"),
            footer_muted(" quit"),
        ]),
        Screen::Ssh => Line::from(vec![
            footer_key("e"),
            footer_muted(" edit Host · "),
            footer_key("i"),
            footer_muted(" install key · "),
            footer_key("Enter"),
            footer_muted(" edit Host · "),
            footer_key("←/→"),
            footer_muted(" tabs · "),
            footer_key("Tab"),
            footer_muted(" dry-run/apply · "),
            footer_key("r"),
            footer_muted(" refresh · "),
            footer_key("?"),
            footer_muted(" help · "),
            footer_key("q"),
            footer_muted(" quit"),
        ]),
        Screen::Overview | Screen::Cli | Screen::Api | Screen::Mcp => Line::from(vec![
            footer_key("Enter"),
            footer_muted(" action · "),
            footer_key("←/→"),
            footer_muted(" tabs · "),
            footer_key("Tab"),
            footer_muted(" dry-run/apply · "),
            footer_key("r"),
            footer_muted(" refresh · "),
            footer_key("?"),
            footer_muted(" help · "),
            footer_key("q"),
            footer_muted(" quit"),
        ]),
        Screen::Reboot => Line::from(vec![
            footer_key("Enter"),
            footer_muted(" reboot · "),
            footer_key("←/→"),
            footer_muted(" tabs · "),
            footer_key("Tab"),
            footer_muted(" dry-run/apply · "),
            footer_key("r"),
            footer_muted(" refresh · "),
            footer_key("?"),
            footer_muted(" help · "),
            footer_key("q"),
            footer_muted(" quit"),
        ]),
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
    panel_text: String,
    modal: Option<Modal>,
    /// After save-token confirm, offer reboot when remote apply installed payload.
    pending_reboot_offer: bool,
    help_open: bool,
    message: String,
    /// Local TUI / tip CLI long version (`LONG_VERSION`).
    cli_local: String,
    /// Last known box CLI status (remote). Starts as [`BoxCliView::Probing`].
    box_cli: BoxCliView,
    /// Box CLI matches `cli_local` (remote mode). Local mode always true.
    cli_current: bool,
    /// Last full surface probe (SSH/CLI/API/MCP).
    surfaces: Option<SurfaceProbeReport>,
    /// Extra Overview lines from doctor/status snapshot.
    overview_extra: String,
    /// Sender half for background remote probes.
    probe_tx: Sender<RemoteProbeOutcome>,
    /// Receiver polled on the UI thread.
    probe_rx: Receiver<RemoteProbeOutcome>,
    /// True while a remote probe thread is still running.
    probe_inflight: bool,
    /// Background reboot result (Ok / Err string).
    reboot_rx: Option<Receiver<std::result::Result<(), String>>>,
    /// True while a reboot SSH thread is still running.
    reboot_inflight: bool,
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
        let (probe_tx, probe_rx) = mpsc::channel();
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
            panel_text: String::new(),
            modal: None,
            pending_reboot_offer: false,
            help_open: false,
            message: String::new(),
            cli_local: LONG_VERSION.to_owned(),
            box_cli: if cli.remote.is_some() {
                BoxCliView::Probing
            } else {
                BoxCliView::Known(RemoteBoxCliStatus::Found(LONG_VERSION.to_owned()))
            },
            cli_current: cli.remote.is_none(),
            surfaces: None,
            overview_extra: String::new(),
            probe_tx,
            probe_rx,
            probe_inflight: false,
            reboot_rx: None,
            reboot_inflight: false,
        };
        if app.remote.is_some() {
            app.rebuild_remote_steps(None);
            app.overview_text =
                tabs::panel_overview_remote(app.remote.as_deref().unwrap_or("?"), None, "");
            app.message = format!("Refreshing {}", app.remote.as_deref().unwrap_or("box"));
            app.refresh_panel_text();
        } else {
            app.refresh();
        }
        app
    }

    fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    fn refresh_panel_text(&mut self) {
        let host = self.remote.as_deref().unwrap_or("local");
        self.panel_text = match self.screen {
            Screen::Ssh => tabs::panel_ssh(
                self.is_remote(),
                host,
                self.surfaces.as_ref(),
                self.box_cli.as_label(),
            ),
            Screen::Cli => tabs::panel_cli(
                self.is_remote(),
                self.surfaces.as_ref(),
                self.box_cli.as_label(),
            ),
            Screen::Api => tabs::panel_api(self.surfaces.as_ref()),
            Screen::Mcp => tabs::panel_mcp(self.surfaces.as_ref()),
            Screen::Reboot => tabs::panel_reboot(host),
            Screen::Overview => {
                if self.is_remote() {
                    tabs::panel_overview_remote(host, self.surfaces.as_ref(), &self.overview_extra)
                } else {
                    self.overview_text.clone()
                }
            }
            Screen::Setup | Screen::Logs => String::new(),
        };
    }

    fn s0_line(&self) -> String {
        let status = match &self.box_cli {
            BoxCliView::Probing => "probing",
            BoxCliView::Known(RemoteBoxCliStatus::AuthFailed)
            | BoxCliView::Known(RemoteBoxCliStatus::Unreachable) => "blocked",
            _ if self.cli_current => "done",
            _ => "pending",
        };
        format!("s0 | {status} | Sync CLI to box")
    }

    fn rebuild_remote_steps(&mut self, setup: Option<&horto_os_ui_shared::SetupStatusReport>) {
        let mut lines = vec![self.s0_line()];
        if let Some(report) = setup {
            lines.extend(report.steps.iter().map(|s| {
                format!(
                    "{} | {} | {}{}",
                    s.id,
                    s.status,
                    s.title,
                    if s.destructive { " *" } else { "" }
                )
            }));
        } else {
            lines.extend(pipeline(self.kind).iter().map(|s| {
                format!(
                    "{} | pending | {}{}",
                    s.id(),
                    s.title(),
                    if s.destructive() { " *" } else { "" }
                )
            }));
        }
        self.status_lines = lines;
        self.select_smart_step();
    }

    fn select_smart_step(&mut self) {
        if self.remote.is_some() && !self.cli_current {
            self.step_state.select(Some(0));
            return;
        }
        let start = if self.remote.is_some() { 1 } else { 0 };
        for (i, line) in self.status_lines.iter().enumerate().skip(start) {
            let status = line.split(" | ").nth(1).unwrap_or("");
            if status == "pending" {
                self.step_state.select(Some(i));
                return;
            }
        }
        if self.status_lines.len() > start {
            self.step_state.select(Some(start));
        } else {
            self.step_state.select(Some(0));
        }
    }

    fn set_pipeline_kind_local(&mut self, kind: SetupKind) {
        self.kind = kind;
        if self.remote.is_some() {
            // Local only: no SSH/SCP. Keep s0 state; reset setup rows to placeholders.
            self.rebuild_remote_steps(None);
            self.message = format!("pipeline={} (press r for status)", kind.as_str());
        } else {
            self.refresh_local();
            self.message = format!("pipeline={}", kind.as_str());
        }
    }

    fn remote_opts(&self) -> Option<RemoteOptions> {
        self.remote.as_ref().map(|host| {
            let mut opts = RemoteOptions {
                host: host.clone(),
                install_ssh_key: self.install_ssh_key,
                bin_dir: self.bin_dir.clone(),
                force_askpass: true,
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

    fn refresh(&mut self) {
        if self.remote.is_some() {
            self.start_remote_probe();
        } else {
            self.refresh_local();
        }
    }

    /// Spawn a background remote probe if none is already running.
    fn start_remote_probe(&mut self) {
        let Some(opts) = self.remote_opts() else {
            return;
        };
        if self.probe_inflight {
            self.message = "Refresh already running".into();
            return;
        }
        let host = opts.host.clone();
        let full = self.kind != SetupKind::Minimal;
        self.probe_inflight = true;
        self.box_cli = BoxCliView::Probing;
        self.surfaces = None;
        self.rebuild_remote_steps(None);
        self.message = format!("Refreshing {host}");
        let tx = self.probe_tx.clone();
        thread::spawn(move || {
            let _ = tx.send(run_remote_probe(opts, full));
        });
    }

    /// Apply any completed background probe without blocking.
    fn poll_probe(&mut self) {
        match self.probe_rx.try_recv() {
            Ok(outcome) => {
                self.probe_inflight = false;
                self.apply_remote_probe(outcome);
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {}
        }
    }

    fn apply_remote_probe(&mut self, outcome: RemoteProbeOutcome) {
        let host = outcome.host;
        let ok = match outcome.result {
            Ok(ok) => ok,
            Err(e) => {
                self.box_cli = BoxCliView::Known(RemoteBoxCliStatus::Unreachable);
                self.cli_current = false;
                self.surfaces = None;
                self.push_log(format!("ERROR probe: {e}"));
                self.message = format!("Refresh failed: {e}");
                self.rebuild_remote_steps(None);
                self.refresh_panel_text();
                return;
            }
        };
        self.apply_remote_probe_ok(&host, ok);
    }

    fn apply_remote_probe_ok(&mut self, host: &str, ok: RemoteProbeOk) {
        self.box_cli = BoxCliView::Known(ok.report.cli.status.clone());
        self.cli_current = ok.report.cli.current;
        self.surfaces = Some(ok.report);
        self.push_log(format!(
            "probe ssh={} cli={} api={}",
            self.surfaces.as_ref().unwrap().ssh.status,
            self.box_cli.as_label(),
            self.surfaces.as_ref().unwrap().api.health
        ));

        self.overview_extra.clear();
        match ok.snapshot {
            Some(Ok(snap)) => {
                self.rebuild_remote_steps(snap.setup.as_ref());
                if let Some(doc) = &snap.doctor {
                    self.overview_extra.push_str(&format!(
                        "doctor: root={} sudo={} docker={} full_env={} minimal_env={}\n",
                        doc.is_root,
                        doc.has_sudo,
                        doc.docker_present,
                        doc.full_env,
                        doc.minimal_env
                    ));
                }
                self.push_log(format!(
                    "setup steps={}",
                    snap.setup.as_ref().map(|s| s.steps.len()).unwrap_or(0)
                ));
            }
            Some(Err(e)) => {
                self.rebuild_remote_steps(None);
                self.push_log(format!("ERROR setup status: {e}"));
            }
            None => {
                self.rebuild_remote_steps(None);
            }
        }

        self.overview_text =
            tabs::panel_overview_remote(host, self.surfaces.as_ref(), &self.overview_extra);
        self.refresh_panel_text();
        if matches!(
            self.box_cli,
            BoxCliView::Known(RemoteBoxCliStatus::AuthFailed)
        ) {
            self.message = "SSH auth failed. Check key or password; UI stays up.".into();
        } else if self.message.starts_with("Refreshing") {
            // End of refresh only: do not wipe unrelated user feedback (reboot, steps, …).
            self.message.clear();
        }
    }

    fn open_host_editor(&mut self) {
        let initial = self.remote.clone().unwrap_or_default();
        self.modal = Some(Modal::TextHost(TextInput::new("OpenSSH Host", initial)));
        self.message = "Edit OpenSSH Host".into();
    }

    fn apply_host_edit(&mut self, host: String) {
        let host = host.trim().to_owned();
        if host.is_empty() {
            self.message = "Host unchanged (empty)".into();
            return;
        }
        self.remote = Some(host.clone());
        self.box_cli = BoxCliView::Probing;
        self.cli_current = false;
        self.surfaces = None;
        self.rebuild_remote_steps(None);
        self.refresh_panel_text();
        self.push_log(format!("host set to {host}; refreshing..."));
        self.start_remote_probe();
    }

    fn install_ssh_key_action(&mut self) {
        if !self.install_ssh_key {
            self.message = "Start with --install-ssh-key to enable key install".into();
            return;
        }
        let Some(opts) = self.remote_opts() else {
            return;
        };
        self.push_log("SSH: installing key…");
        match horto_os_ui_shared::remote_ensure_ssh_key(&SystemProcessRunner, &opts) {
            Ok(()) => {
                self.message = "SSH key installed (or already authorized)".into();
                self.refresh();
            }
            Err(e) => {
                self.push_log(format!("ERROR ssh key: {e}"));
                self.message = format!("SSH key failed: {e}");
            }
        }
    }

    fn run_surface_enter(&mut self) {
        match self.screen {
            Screen::Ssh => self.open_host_editor(),
            Screen::Cli => self.run_s0_sync(),
            Screen::Api | Screen::Mcp | Screen::Overview => self.refresh(),
            Screen::Reboot => self.arm_reboot_confirm(),
            Screen::Setup | Screen::Logs => {}
        }
    }

    fn arm_reboot_confirm(&mut self) {
        self.modal = Some(Modal::Confirm(ConfirmKind::Reboot));
    }

    fn note(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }

    fn do_reboot(&mut self, sudo_password: String) {
        if self.reboot_inflight {
            self.note("Reboot already running");
            return;
        }
        let Some(opts) = self.remote_opts() else {
            return;
        };
        let host = opts.host.clone();
        self.push_log("reboot: sudo reboot on box…");
        self.modal = Some(Modal::Rebooting);
        self.note("Rebooting…");
        self.reboot_inflight = true;
        let (tx, rx) = mpsc::channel();
        self.reboot_rx = Some(rx);
        thread::spawn(move || {
            let result = horto_os_ui_shared::remote_reboot_with_sudo_password(
                &SystemProcessRunner,
                &opts,
                &sudo_password,
            );
            match result {
                Ok(()) => {
                    // SSH returned; keep UI on Rebooting until the box stops answering.
                    wait_until_host_down(&host, Duration::from_secs(90));
                    let _ = tx.send(Ok(()));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        });
    }

    fn poll_reboot(&mut self) {
        let Some(rx) = &self.reboot_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.reboot_inflight = false;
                self.reboot_rx = None;
                if matches!(self.modal, Some(Modal::Rebooting)) {
                    self.modal = None;
                }
                self.push_log("Reboot issued");
                self.note("Reboot issued");
            }
            Ok(Err(e)) => {
                self.reboot_inflight = false;
                self.reboot_rx = None;
                if matches!(self.modal, Some(Modal::Rebooting)) {
                    self.modal = None;
                }
                self.push_log(format!("ERROR reboot: {e}"));
                self.note(short_reboot_err(&e));
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.reboot_inflight = false;
                self.reboot_rx = None;
                if matches!(self.modal, Some(Modal::Rebooting)) {
                    self.modal = None;
                }
                self.note("Reboot failed (worker dropped)");
            }
        }
    }

    fn resolve_confirm_yes(&mut self) {
        let Some(Modal::Confirm(kind)) = self.modal.take() else {
            return;
        };
        match kind {
            ConfirmKind::DestructiveStep(id) => self.execute_step(&id),
            ConfirmKind::Reboot | ConfirmKind::RebootAfterApply => {
                if self.dry_run {
                    self.note("DRY-RUN: reboot not sent (Tab → APPLY)");
                    self.push_log("DRY-RUN: reboot not sent (press Tab for APPLY)");
                    return;
                }
                self.modal = Some(Modal::SudoPassword(SecretInput::new("Sudo password (box)")));
                self.message = "Enter sudo password".into();
            }
            ConfirmKind::SaveToken(token) => {
                match finish_save_api_token(&token, "y") {
                    Ok(true) => {
                        self.push_log(
                            "Saved status-api bearer to ~/.config/horto-os-ui/api_token".to_owned(),
                        );
                        self.message = "API token saved".into();
                    }
                    Ok(false) => {
                        self.push_log("Skipped saving status-api bearer locally");
                        self.message = "Token save skipped".into();
                    }
                    Err(e) => {
                        self.push_log(format!("Token save failed: {e}"));
                        self.message = format!("Token save failed: {e}");
                    }
                }
                self.maybe_offer_reboot_after_token();
            }
        }
    }

    fn resolve_confirm_no(&mut self) {
        let Some(Modal::Confirm(kind)) = self.modal.take() else {
            self.modal = None;
            self.message = "Cancelled".into();
            return;
        };
        self.message = "Cancelled".into();
        if matches!(kind, ConfirmKind::SaveToken(_)) {
            self.push_log("Skipped saving status-api bearer locally");
            self.maybe_offer_reboot_after_token();
        }
    }

    fn maybe_offer_reboot_after_token(&mut self) {
        if !self.pending_reboot_offer {
            return;
        }
        self.pending_reboot_offer = false;
        self.modal = Some(Modal::Confirm(ConfirmKind::RebootAfterApply));
        self.message = "Reboot after apply? Enter/y confirm, Esc/n cancel.".into();
    }

    fn run_s0_sync(&mut self) {
        let Some(opts) = self.remote_opts() else {
            return;
        };
        self.push_log("s0: syncing CLI to box…");
        match remote_upload_cli(&SystemProcessRunner, &opts) {
            Ok(probe) => {
                self.box_cli = BoxCliView::Known(probe.status.clone());
                self.cli_current = probe.current;
                self.push_log(format!("s0: box={}", probe.status.as_label()));
                if probe.current {
                    self.refresh();
                    self.message = "s0 done; refreshing...".into();
                } else {
                    self.rebuild_remote_steps(None);
                    self.message = format!(
                        "s0 finished but box still {}; check SSH/auth",
                        probe.status.as_label()
                    );
                }
            }
            Err(e) => {
                self.push_log(format!("ERROR s0: {e}"));
                self.message = format!("s0 failed: {e}");
            }
        }
    }

    fn refresh_local(&mut self) {
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
        overview.push_str("Mode: embedded\n");
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
        let opts = RemoteOptions::default();
        match probe_surfaces(&SystemProcessRunner, &opts, true) {
            Ok(report) => {
                self.surfaces = Some(report);
            }
            Err(e) => {
                self.push_log(format!("ERROR local probe: {e}"));
            }
        }
        self.refresh_panel_text();
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
                offer_reboot_on_success: false,
                capture_output: false,
            },
        ) {
            Ok(outcome) => {
                for line in outcome.log.lines() {
                    self.push_log(line.to_owned());
                }
                let offer_reboot = install_payload && !self.dry_run;
                if let Some(token) = outcome.api_token {
                    self.pending_reboot_offer = offer_reboot;
                    self.modal = Some(Modal::Confirm(ConfirmKind::SaveToken(token)));
                    self.message = "Save API token? Enter/y confirm, Esc/n cancel.".into();
                } else if offer_reboot {
                    self.modal = Some(Modal::Confirm(ConfirmKind::RebootAfterApply));
                    self.message = "Reboot after apply? Enter/y confirm, Esc/n cancel.".into();
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

    fn selected_step_id(&self) -> Option<String> {
        let idx = self.step_state.selected()?;
        self.status_lines
            .get(idx)
            .and_then(|l| l.split(" | ").next())
            .map(str::to_string)
    }

    fn push_log(&mut self, line: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.logs.push(format!("{ts} {}", line.into()));
        if self.logs.len() > 500 {
            self.logs.drain(0..self.logs.len() - 500);
        }
    }

    fn clear_logs(&mut self) {
        self.logs.clear();
        self.message = "Logs cleared".into();
    }

    fn next_screen(&mut self) {
        self.screen = self.screen.next(self.is_remote());
        self.refresh_panel_text();
    }

    fn prev_screen(&mut self) {
        self.screen = self.screen.prev(self.is_remote());
        self.refresh_panel_text();
    }

    fn select_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.help_open = false;
        self.refresh_panel_text();
    }

    fn run_selected(&mut self) {
        let Some(id) = self.selected_step_id() else {
            return;
        };
        if id == "s0" {
            self.run_s0_sync();
            return;
        }
        if self.remote.is_some() && !self.cli_current {
            self.message = match &self.box_cli {
                BoxCliView::Probing => "Still probing box CLI...".into(),
                BoxCliView::Known(RemoteBoxCliStatus::AuthFailed) => {
                    "SSH auth failed: install key (--install-ssh-key) or run s0 after login".into()
                }
                BoxCliView::Known(RemoteBoxCliStatus::Unreachable) => {
                    "Box unreachable: check host/network, then press r".into()
                }
                _ => "Run s0 (Sync CLI) before other steps".into(),
            };
            return;
        }
        let ctx_probe = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx_probe, self.kind);
        if let Some(row) = report.steps.iter().find(|s| s.id == id) {
            if row.destructive && !self.dry_run {
                self.modal = Some(Modal::Confirm(ConfirmKind::DestructiveStep(id)));
                self.message = "Step is destructive. Enter/y confirm, Esc/n cancel.".into();
                return;
            }
        }
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
        if self.remote.is_some() && !self.cli_current {
            self.message = match &self.box_cli {
                BoxCliView::Probing => "Still probing box CLI...".into(),
                _ => "Run s0 (Sync CLI) before running all steps".into(),
            };
            return;
        }
        self.push_log(format!(
            "Running all pipeline steps (dry_run={})",
            self.dry_run
        ));
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
        self.refresh_panel_text();
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    install_signal_handlers();
    install_panic_hook();
    let (_guard, mut terminal) = TerminalGuard::enter()?;
    let mut app = App::new(&cli);
    run_app(&mut terminal, &mut app)
}

fn ctrl_c_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut boot_remote_probe = app.is_remote();
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(());
        }
        app.poll_probe();
        app.poll_reboot();
        terminal.draw(|f| ui(f, app))?;
        if boot_remote_probe {
            app.start_remote_probe();
            boot_remote_probe = false;
        }
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if ctrl_c_quit(key) {
            return Ok(());
        }
        if handle_modal_key(app, key) {
            continue;
        }
        if is_quit(key) {
            return Ok(());
        }
        if app.help_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    app.help_open = false;
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
            KeyCode::Tab => {
                app.dry_run = !app.dry_run;
                app.message = if app.dry_run {
                    "DRY-RUN".into()
                } else {
                    "APPLY".into()
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
            KeyCode::Char('a') => app.run_all(),
            KeyCode::Char('b') => app.run_backup_etc(),
            KeyCode::Char('B') => app.show_disk_backup_status(),
            KeyCode::Char('p') | KeyCode::Char('P') => {
                let next = if app.kind == SetupKind::Full {
                    SetupKind::Minimal
                } else {
                    SetupKind::Full
                };
                app.set_pipeline_kind_local(next);
            }
            KeyCode::Char('e') | KeyCode::Char('E') if app.screen == Screen::Ssh => {
                app.open_host_editor();
            }
            KeyCode::Char('i') | KeyCode::Char('I') if app.screen == Screen::Ssh => {
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
}

/// Handle confirm / text / secret modals. Returns true when the key was consumed.
fn handle_modal_key(app: &mut App, key: KeyEvent) -> bool {
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
                    app.apply_host_edit(value);
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
        Some(Modal::Rebooting) => true,
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

/// Tab title with digit hotkey in magenta (selection highlight overrides when active).
fn tab_title_line(title: &str) -> Line<'static> {
    let mut chars = title.chars();
    let Some(digit) = chars.next() else {
        return Line::from(title.to_owned());
    };
    let rest: String = chars.collect();
    Line::from(vec![
        Span::styled(
            digit.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(rest),
    ])
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

    let remote = app.is_remote();
    let titles = Screen::titles(remote)
        .into_iter()
        .map(tab_title_line)
        .collect::<Vec<_>>();
    let idx = app.screen.index(remote);
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(
            Block::default()
                .borders(Borders::ALL)
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
        Screen::Overview
        | Screen::Ssh
        | Screen::Cli
        | Screen::Api
        | Screen::Mcp
        | Screen::Reboot => draw_panel(f, app, chunks[1]),
    }

    draw_footer(f, app, chunks[2]);

    if app.help_open {
        draw_help(f);
    }
    match &app.modal {
        Some(Modal::Confirm(kind)) => {
            draw_confirm(f, kind.title(), &kind.body());
        }
        Some(Modal::TextHost(input)) => {
            draw_text_input(f, input);
        }
        Some(Modal::SudoPassword(input)) => {
            draw_secret_input(f, input);
        }
        Some(Modal::Rebooting) => {
            draw_rebooting(f);
        }
        None => {}
    }
}

/// Footer: status line + keys line. Cleared each frame so shorter text leaves no garbage.
fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Status");
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let status = pad_footer_line(footer_status_line(app), inner.width);
    f.render_widget(
        Paragraph::new(status),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );
    if inner.height >= 2 {
        let hints = pad_footer_line(footer_hints_line(app), inner.width);
        f.render_widget(
            Paragraph::new(hints),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn pad_footer_line(line: Line<'static>, width: u16) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::default();
    }
    let used = line.width();
    if used > width {
        let s = line.to_string();
        let mut out = String::new();
        for ch in s.chars() {
            if out.chars().count() + 1 >= width {
                break;
            }
            out.push(ch);
        }
        if out.chars().count() == width {
            out.pop();
            out.push('…');
        }
        // Keep user-facing Status text bright white after truncate.
        return Line::from(Span::styled(
            out,
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if used < width {
        let mut line = line;
        line.spans.push(Span::raw(" ".repeat(width - used)));
        return line;
    }
    line
}

fn short_reboot_err(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    if lower.contains("sorry, try again") || lower.contains("incorrect password") {
        return "Reboot failed: wrong sudo password".into();
    }
    if lower.contains("no password was provided") || lower.contains("a terminal is required") {
        return "Reboot failed: sudo password not accepted".into();
    }
    let one_line: String = err
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let trimmed = one_line.trim();
    if trimmed.len() > 80 {
        format!("Reboot failed: {}…", &trimmed[..77])
    } else {
        format!("Reboot failed: {trimmed}")
    }
}

/// Poll until SSH to `host` fails (box going down) or `timeout` elapses.
fn wait_until_host_down(host: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let status = std::process::Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=3",
                "-o",
                "StrictHostKeyChecking=accept-new",
                host,
                "true",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let down = !matches!(status, Ok(s) if s.success());
        if down || Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(800));
    }
}

fn draw_help(f: &mut Frame) {
    let area = centered_rect(70, 80, f.area());
    f.render_widget(Clear, area);
    let body = [
        "Horto TUI help",
        "",
        "q / Esc / Ctrl+C   Quit",
        "?                  Toggle this help",
        "Left / Right       Previous / next tab",
        "1-8                Jump to tab (remote: 7 Reboot, 8 Logs)",
        "j k / Up / Down    Move step selection (Setup)",
        "p                  Toggle full / minimal pipeline",
        "Tab                Toggle dry-run / apply",
        "Enter              Setup: run step · SSH: edit Host · other surfaces: action",
        "e / i              SSH: edit Host / install key (--install-ssh-key)",
        "a                  Run all pipeline steps",
        "b / B              Timestamped /etc backup / disk probe",
        "r                  Refresh surfaces (SSH/CLI/API/MCP; background)",
        "c                  Clear Logs (on Logs tab)",
        "y / n              Confirm / cancel (modals)",
        "",
        "Remote open paints first, then refreshes SSH/CLI/API/MCP in the background.",
        "Press r to refresh without blocking. Confirm and Host edit use on-screen dialogs.",
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
        .map(|l| ListItem::new(setup_step_line(l)))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(vec![
                    panel_title("Steps"),
                    Span::raw(" ("),
                    footer_key("Enter"),
                    Span::raw(" run · "),
                    footer_key("a"),
                    Span::raw(" = all · "),
                    footer_key("*"),
                    Span::raw(" = destructive)"),
                ])),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, area, &mut app.step_state);
}

/// Color step id and status separately: `s0 | pending | title`.
fn setup_step_line(raw: &str) -> Line<'static> {
    let parts: Vec<&str> = raw.splitn(3, " | ").collect();
    if parts.len() < 3 {
        return Line::from(Span::raw(raw.to_owned()));
    }
    let id = parts[0];
    let status = parts[1];
    let title = parts[2];
    let status_style = match status {
        "done" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        "pending" => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        "probing" => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        "stale" => Style::default()
            .fg(Color::Rgb(255, 165, 0))
            .add_modifier(Modifier::BOLD),
        "failed" | "blocked" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    };
    Line::from(vec![
        Span::styled(
            id.to_owned(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(status.to_owned(), status_style),
        Span::raw(" | "),
        Span::styled(title.to_owned(), Style::default().fg(Color::Gray)),
    ])
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let n = app.logs.len();
    let text = if app.logs.is_empty() {
        "(empty - press r to refresh · c to clear)".to_string()
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
    let p = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Line::from(vec![
                panel_title("Logs"),
                Span::raw(format!(" ({n})")),
            ])),
    );
    f.render_widget(p, area);
}

fn draw_panel(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.screen {
        Screen::Overview => "Overview",
        Screen::Ssh => "SSH",
        Screen::Cli => "CLI",
        Screen::Api => "API",
        Screen::Mcp => "MCP",
        Screen::Reboot => "Reboot",
        Screen::Setup | Screen::Logs => "",
    };
    let body = if app.screen == Screen::Overview && !app.is_remote() {
        app.overview_text.clone()
    } else {
        app.panel_text.clone()
    };
    let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Line::from(panel_title(title))),
    );
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
