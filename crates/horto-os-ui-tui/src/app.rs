//! TUI application state and setup/apply actions.

use clap::Parser;
use horto_os_ui_shared::{
    backup_etc_timestamped, box_status, finish_save_api_token,
    install_ecosystem_after_embedded_apply, pipeline, probe_api_surface, probe_cli_surface,
    probe_disk_backup, probe_mcp_surface, probe_ssh_surface, probe_surfaces, remote_run_cli,
    remote_upload_cli, require_root_for_apply, setup_run, setup_step, ApiSurfaceProbe, ApplyMode,
    CliSurfaceProbe, DiskBackupOpts, EcosystemInstallChoice, HostContext, McpHostProbe,
    RemoteBoxCliStatus, RemoteOptions, RemoteOptionsInput, RemoteRunFlags, RemoteRunRequest,
    SetupKind, SshSurfaceProbe, StackOpts, StdioPrompts, SurfaceProbeReport, SystemProcessRunner,
    DEFAULT_INSTALL_DIR, LONG_VERSION,
};
use ratatui::text::Line;
use ratatui::widgets::ListState;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::probe_job::{run_remote_probe, RemoteProbeOk, RemoteProbeOutcome};
use crate::prompt::{SecretInput, TextInput};
use crate::tabs::{self, Screen};

/// In-TUI overlay (confirm / free-text / sudo password).
#[derive(Debug, Clone)]
pub enum Modal {
    Confirm(ConfirmKind),
    TextHost(TextInput),
    SudoPassword(SecretInput),
    /// Wait while reboot SSH runs on a background thread.
    Rebooting,
}

/// Progress from the background reboot thread.
pub enum RebootEvent {
    /// `sudo reboot` accepted over SSH.
    Issued,
    /// Box answers SSH again.
    BoxBack,
    Failed(String),
}

/// Result of `f` fetch for one surface tab (not a full `r` refresh).
pub enum FetchEvent {
    Overview(Box<SurfaceProbeReport>),
    Ssh(SshSurfaceProbe),
    Cli(CliSurfaceProbe),
    Api(ApiSurfaceProbe),
    Mcp(McpHostProbe, McpHostProbe),
    Failed(String),
}

/// Confirmation dialog kinds shown as TUI overlays.
#[derive(Debug, Clone)]
pub enum ConfirmKind {
    DestructiveStep(String),
    Reboot,
    SaveToken(String),
    RebootAfterApply,
    /// Ask whether to install status-api after full apply.
    InstallStatusApi {
        remote: bool,
    },
    /// Ask whether to install MCP (after status-api answer).
    InstallMcp {
        remote: bool,
        status_api: bool,
    },
}

impl ConfirmKind {
    /// Short title for the confirm modal chrome.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::DestructiveStep(_) => "Confirm destructive step",
            Self::Reboot => "Confirm reboot",
            Self::SaveToken(_) => "Save API token",
            Self::RebootAfterApply => "Reboot after apply",
            Self::InstallStatusApi { .. } => "Install status-api?",
            Self::InstallMcp { .. } => "Install MCP?",
        }
    }

    /// Operator-facing body text for the confirm modal.
    #[must_use]
    pub fn body(&self) -> String {
        match self {
            Self::DestructiveStep(id) => {
                format!("Step {id} is destructive. Continue?")
            }
            Self::Reboot => "Reboot the box now?".into(),
            Self::SaveToken(_) => {
                "Save status-api bearer to ~/.config/horto-os-ui/api_token?".into()
            }
            Self::RebootAfterApply => "Remote apply succeeded. Reboot the box now?".into(),
            Self::InstallStatusApi { remote } => {
                if *remote {
                    "Install status-api (systemd) on the remote box?".into()
                } else {
                    "Install status-api (systemd) on this host?".into()
                }
            }
            Self::InstallMcp { remote, .. } => {
                if *remote {
                    "Install MCP (systemd) on the remote box?".into()
                } else {
                    "Install MCP (systemd) on this host?".into()
                }
            }
        }
    }
}

/// Box CLI footer/overview value (remote mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoxCliView {
    /// Probe in flight or not yet applied (never shown as missing).
    Probing,
    /// Last probe result from shared remote layer.
    Known(RemoteBoxCliStatus),
}

impl BoxCliView {
    /// Compact footer/overview label (`probing...`, auth failed, version, …).
    #[must_use]
    pub const fn as_label(&self) -> &str {
        match self {
            Self::Probing => "probing...",
            Self::Known(s) => s.as_label(),
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-tui",
    about = "Horto OS UI terminal",
    version,
    long_version = LONG_VERSION
)]
/// Clap flags for the TUI binary (`--remote`, `--apply`, pipeline toggles).
pub struct Cli {
    #[arg(long)]
    pub apply: bool,
    #[arg(long)]
    pub minimal: bool,
    #[arg(long)]
    pub skip_piper: bool,
    /// Optional Docker stacks CSV for d3
    #[arg(long, default_value = "", env = "HORTO_STACKS")]
    pub stacks: String,
    /// OpenSSH Host alias, /etc/hosts name, or user@host; run setup via remote runner
    #[arg(long)]
    pub remote: Option<String>,
    /// Opt-in: install this PC's public key on the box. Off by default.
    #[arg(long, default_value_t = false)]
    pub install_ssh_key: bool,
    /// Local directory with box binaries (skips GitHub Release download)
    #[arg(long, env = "HORTO_BIN_DIR")]
    pub bin_dir: Option<std::path::PathBuf>,
    /// GitHub Release tag for box tar.gz (`v0.1.0` or tip `dev-preview`)
    #[arg(long, env = "HORTO_RELEASE_TAG")]
    pub release_tag: Option<String>,
}

/// Mutable TUI session: screens, setup list, logs, modals, and background probe/fetch/reboot.
#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub screen: Screen,
    pub apply: bool,
    pub kind: SetupKind,
    pub skip_piper: bool,
    /// Optional Docker stacks for `d3`.
    pub stack_opts: StackOpts,
    pub remote: Option<String>,
    pub install_ssh_key: bool,
    pub bin_dir: Option<std::path::PathBuf>,
    pub release_tag: Option<String>,
    pub step_state: ListState,
    pub logs: Vec<String>,
    pub status_lines: Vec<String>,
    pub overview_text: String,
    pub panel_lines: Vec<Line<'static>>,
    pub modal: Option<Modal>,
    /// After save-token confirm, offer reboot when remote apply installed payload.
    pub pending_reboot_offer: bool,
    pub help_open: bool,
    pub message: String,
    /// Local TUI / tip CLI long version (`LONG_VERSION`).
    pub cli_local: String,
    /// Last known box CLI status (remote). Starts as [`BoxCliView::Probing`].
    pub box_cli: BoxCliView,
    /// Box CLI matches `cli_local` (remote mode). Local mode always true.
    pub cli_current: bool,
    /// Last full surface probe (SSH/CLI/API/MCP).
    pub surfaces: Option<SurfaceProbeReport>,
    /// Extra Overview lines from doctor/status snapshot.
    pub overview_extra: String,
    /// Sender half for background remote probes.
    pub probe_tx: Sender<RemoteProbeOutcome>,
    /// Receiver polled on the UI thread.
    pub probe_rx: Receiver<RemoteProbeOutcome>,
    /// True while a remote probe thread is still running.
    pub probe_inflight: bool,
    /// Background reboot progress.
    pub reboot_rx: Option<Receiver<RebootEvent>>,
    /// True while a reboot SSH thread is still running.
    pub reboot_inflight: bool,
    /// Set by Esc to stop the reboot wait thread.
    pub reboot_cancel: Arc<AtomicBool>,
    /// Background single-tab fetch (`f`).
    pub fetch_rx: Option<Receiver<FetchEvent>>,
    pub fetch_inflight: bool,
    /// `/etc/hosts` + SSH config names for the Host editor (Tab cycles).
    pub host_candidates: Vec<String>,
    /// Index into [`Self::host_candidates`] for Tab cycling.
    pub host_candidate_idx: usize,
}

impl App {
    /// Build app state from clap flags and kick off local refresh or remote probe.
    pub fn new(cli: &Cli) -> Self {
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
            apply: cli.apply,
            kind,
            skip_piper: cli.skip_piper,
            stack_opts: StackOpts::parse_csv(&cli.stacks),
            remote: cli.remote.clone(),
            install_ssh_key: cli.install_ssh_key,
            bin_dir: cli.bin_dir.clone(),
            release_tag: cli.release_tag.clone(),
            step_state,
            logs: Vec::new(),
            status_lines: Vec::new(),
            overview_text: String::new(),
            panel_lines: Vec::new(),
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
            reboot_cancel: Arc::new(AtomicBool::new(false)),
            fetch_rx: None,
            fetch_inflight: false,
            host_candidates: Vec::new(),
            host_candidate_idx: 0,
        };
        if app.remote.is_some() {
            app.rebuild_remote_steps(None);
            app.message = format!("Refreshing {}", app.remote.as_deref().unwrap_or("box"));
            app.refresh_panel_text();
        } else {
            app.refresh();
        }
        app
    }
    /// True when a remote OpenSSH host is configured.
    pub const fn is_remote(&self) -> bool {
        self.remote.is_some()
    }
    /// Rebuild colored panel lines for the active surface tab.
    pub fn refresh_panel_text(&mut self) {
        let host = self.remote.as_deref().unwrap_or("local");
        self.panel_lines = match self.screen {
            Screen::Ssh => tabs::panel_ssh(
                self.is_remote(),
                host,
                self.surfaces.as_ref(),
                self.box_cli.as_label(),
                self.install_ssh_key,
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
                    tabs::panel_lines_from_plain(&self.overview_text)
                }
            }
            Screen::Setup | Screen::Logs => Vec::new(),
        };
    }
    /// Setup-list row for remote CLI sync (`s0`).
    pub fn s0_line(&self) -> String {
        let status = match &self.box_cli {
            BoxCliView::Probing => "probing",
            BoxCliView::Known(RemoteBoxCliStatus::AuthFailed | RemoteBoxCliStatus::Unreachable) => {
                "blocked"
            }
            BoxCliView::Known(_) if self.cli_current => "done",
            BoxCliView::Known(_) => "pending",
        };
        format!("s0 | {status} | Sync CLI to box")
    }
    /// Rebuild the setup list from an optional box status report (or placeholders).
    pub fn rebuild_remote_steps(&mut self, setup: Option<&horto_os_ui_shared::SetupStatusReport>) {
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
    /// Select the first pending setup row (prefer `s0` when box CLI is stale).
    pub fn select_smart_step(&mut self) {
        if self.remote.is_some() && !self.cli_current {
            self.step_state.select(Some(0));
            return;
        }
        let start = usize::from(self.remote.is_some());
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
    /// Switch full/minimal pipeline locally without SSH; refresh UI text.
    pub fn set_pipeline_kind_local(&mut self, kind: SetupKind) {
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
    /// Build [`RemoteOptions`] from current host / key / bin-dir settings.
    pub fn remote_opts(&self) -> Option<RemoteOptions> {
        self.remote.as_ref().map(|host| {
            RemoteOptions::from_input(RemoteOptionsInput {
                host: host.clone(),
                install_ssh_key: self.install_ssh_key,
                bin_dir: self.bin_dir.clone(),
                release_tag: self.release_tag.clone(),
                force_askpass: true,
            })
        })
    }
    /// Refresh local status or start a background remote probe.
    pub fn refresh(&mut self) {
        if self.remote.is_some() {
            self.start_remote_probe();
        } else {
            self.refresh_local();
        }
    }

    /// Spawn a background remote probe if none is already running.
    pub fn start_remote_probe(&mut self) {
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
            let _ = tx.send(run_remote_probe(&opts, full));
        });
    }

    /// Apply any completed background probe without blocking.
    pub fn poll_probe(&mut self) {
        match self.probe_rx.try_recv() {
            Ok(outcome) => {
                self.probe_inflight = false;
                self.apply_remote_probe(outcome);
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {}
        }
    }
    /// Apply a finished remote probe outcome (success or unreachable error).
    pub fn apply_remote_probe(&mut self, outcome: RemoteProbeOutcome) {
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
    /// Merge a successful remote probe into box CLI, surfaces, and setup rows.
    pub fn apply_remote_probe_ok(&mut self, _host: &str, ok: RemoteProbeOk) {
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
                    let _ = writeln!(
                        self.overview_extra,
                        "doctor: root={} sudo={} docker={} full_env={} minimal_env={}",
                        doc.is_root,
                        doc.has_sudo,
                        doc.docker_present,
                        doc.full_env,
                        doc.minimal_env
                    );
                }
                self.push_log(format!(
                    "setup steps={}",
                    snap.setup.as_ref().map_or(0, |s| s.steps.len())
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
    /// Open the OpenSSH Host text modal.
    pub fn open_host_editor(&mut self) {
        let initial = self.remote.clone().unwrap_or_default();
        self.host_candidates = horto_os_ui_shared::list_known_remote_hosts()
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.name)
            .collect();
        self.host_candidate_idx = self
            .host_candidates
            .iter()
            .position(|n| n == &initial)
            .unwrap_or(0);
        self.modal = Some(Modal::TextHost(TextInput::new("OpenSSH Host", initial)));
        if self.host_candidates.is_empty() {
            self.message = "Edit host (no /etc/hosts or SSH config names)".into();
        } else {
            self.message = format!(
                "Edit host · Tab cycles: {}",
                self.host_candidates.join(", ")
            );
        }
    }

    /// Cycle the Host editor buffer through known `/etc/hosts` + SSH config names.
    pub fn cycle_host_candidate(&mut self) {
        let Some(Modal::TextHost(input)) = self.modal.as_mut() else {
            return;
        };
        if self.host_candidates.is_empty() {
            self.message = "No known hosts in /etc/hosts or SSH config".into();
            return;
        }
        if !input.buffer().is_empty() {
            self.host_candidate_idx = (self.host_candidate_idx + 1) % self.host_candidates.len();
        }
        let name = self.host_candidates[self.host_candidate_idx].clone();
        input.set_buffer(name.clone());
        self.message = format!(
            "Host candidate {}/{}: {name}",
            self.host_candidate_idx + 1,
            self.host_candidates.len()
        );
    }
    /// Apply a host edit and start a remote probe for the new host.
    pub fn apply_host_edit(&mut self, host: &str) {
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
    /// Run opt-in `ssh-copy-id` when `--install-ssh-key` was set.
    pub fn install_ssh_key_action(&mut self) {
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
    /// Enter-key action for the current surface tab (SSH host, CLI sync, reboot).
    pub fn run_surface_enter(&mut self) {
        match self.screen {
            Screen::Ssh => self.open_host_editor(),
            Screen::Cli => self.run_s0_sync(),
            Screen::Reboot => self.arm_reboot_confirm(),
            Screen::Overview | Screen::Api | Screen::Mcp | Screen::Setup | Screen::Logs => {}
        }
    }
    /// Show the reboot confirmation modal.
    pub fn arm_reboot_confirm(&mut self) {
        self.modal = Some(Modal::Confirm(ConfirmKind::Reboot));
    }
    /// Set the footer status message.
    pub fn note(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
    }
    /// Start background `sudo reboot` with the collected password.
    pub fn do_reboot(&mut self, sudo_password: String) {
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
        self.reboot_inflight = true;
        self.reboot_cancel.store(false, Ordering::SeqCst);
        let cancel = Arc::clone(&self.reboot_cancel);
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
                    let _ = tx.send(RebootEvent::Issued);
                    match wait_until_box_replies(&host, &cancel) {
                        Ok(()) => {
                            let _ = tx.send(RebootEvent::BoxBack);
                        }
                        Err(e) => {
                            let _ = tx.send(RebootEvent::Failed(e));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(RebootEvent::Failed(e.to_string()));
                }
            }
        });
    }
    /// Drain reboot worker events into logs and modal state.
    pub fn poll_reboot(&mut self) {
        let Some(rx) = &self.reboot_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(RebootEvent::Issued) => {
                self.push_log("Reboot issued");
                self.note("Reboot issued");
            }
            Ok(RebootEvent::BoxBack) => {
                self.reboot_inflight = false;
                self.reboot_rx = None;
                if matches!(self.modal, Some(Modal::Rebooting)) {
                    self.modal = None;
                }
                self.push_log("Reboot done");
                self.note("Reboot done");
            }
            Ok(RebootEvent::Failed(e)) => {
                self.reboot_inflight = false;
                self.reboot_rx = None;
                if matches!(self.modal, Some(Modal::Rebooting)) {
                    self.modal = None;
                }
                if e == "cancelled" {
                    return;
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
    /// Background-fetch the active surface tab (`f` key).
    pub fn fetch_current_tab(&mut self) {
        if !matches!(
            self.screen,
            Screen::Overview | Screen::Ssh | Screen::Cli | Screen::Api | Screen::Mcp
        ) {
            return;
        }
        if self.fetch_inflight || self.probe_inflight {
            self.note("Fetch already running");
            return;
        }
        let Some(opts) = self.remote_opts() else {
            self.note("Fetch needs --remote");
            return;
        };
        let screen = self.screen;
        self.fetch_inflight = true;
        self.note(match screen {
            Screen::Overview => "Fetching overview…",
            Screen::Ssh => "Fetching SSH…",
            Screen::Cli => "Fetching CLI…",
            Screen::Api => "Fetching API…",
            Screen::Mcp => "Fetching MCP…",
            _ => "Fetching…",
        });
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        thread::spawn(move || {
            let embedded = false;
            let event = match screen {
                Screen::Overview => match probe_surfaces(&SystemProcessRunner, &opts, embedded) {
                    Ok(r) => FetchEvent::Overview(Box::new(r)),
                    Err(e) => FetchEvent::Failed(e.to_string()),
                },
                Screen::Ssh => match probe_ssh_surface(&SystemProcessRunner, &opts, embedded) {
                    Ok(r) => FetchEvent::Ssh(r),
                    Err(e) => FetchEvent::Failed(e.to_string()),
                },
                Screen::Cli => match probe_cli_surface(&SystemProcessRunner, &opts, embedded) {
                    Ok(r) => FetchEvent::Cli(r),
                    Err(e) => FetchEvent::Failed(e.to_string()),
                },
                Screen::Api => match probe_api_surface(&SystemProcessRunner, &opts, embedded) {
                    Ok(r) => FetchEvent::Api(r),
                    Err(e) => FetchEvent::Failed(e.to_string()),
                },
                Screen::Mcp => match probe_mcp_surface(&SystemProcessRunner, &opts, embedded) {
                    Ok((pc, bx)) => FetchEvent::Mcp(pc, bx),
                    Err(e) => FetchEvent::Failed(e.to_string()),
                },
                _ => FetchEvent::Failed("unsupported tab".into()),
            };
            let _ = tx.send(event);
        });
    }
    /// Ensure a placeholder [`SurfaceProbeReport`] exists before partial fetches.
    pub fn ensure_surfaces_shell(&mut self) {
        if self.surfaces.is_some() {
            return;
        }
        let host = self.remote.clone().unwrap_or_else(|| "box".into());
        self.surfaces = Some(SurfaceProbeReport {
            local_version: LONG_VERSION.to_owned(),
            ssh: SshSurfaceProbe {
                host: host.clone(),
                status: "…".into(),
                key_ok: false,
            },
            cli: CliSurfaceProbe {
                status: RemoteBoxCliStatus::Unreachable,
                version: None,
                current: false,
            },
            api: ApiSurfaceProbe {
                url: format!("http://{host}:8787"),
                health: "…".into(),
                status: "…".into(),
                local_token: false,
                unit: String::new(),
            },
            mcp_pc: McpHostProbe {
                docker: None,
                binary: None,
                http_url: format!("http://{host}:8790"),
                http_reach: "…".into(),
                unit: String::new(),
                api_health: "…".into(),
            },
            mcp_box: McpHostProbe {
                docker: None,
                binary: None,
                http_url: format!("http://{host}:8790"),
                http_reach: "…".into(),
                unit: String::new(),
                api_health: "…".into(),
            },
        });
    }
    /// Drain single-tab fetch results into surfaces and panels.
    pub fn poll_fetch(&mut self) {
        let Some(rx) = &self.fetch_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(event) => {
                self.fetch_inflight = false;
                self.fetch_rx = None;
                match event {
                    FetchEvent::Overview(report) => {
                        self.box_cli = BoxCliView::Known(report.cli.status.clone());
                        self.cli_current = report.cli.current;
                        self.push_log(format!(
                            "fetch overview ssh={} api={} mcp={}",
                            report.ssh.status, report.api.health, report.mcp_box.http_reach
                        ));
                        self.surfaces = Some(*report);
                        self.note("Overview fetched");
                    }
                    FetchEvent::Ssh(ssh) => {
                        self.ensure_surfaces_shell();
                        if let Some(s) = self.surfaces.as_mut() {
                            s.ssh = ssh;
                        }
                        self.push_log(format!(
                            "fetch ssh={}",
                            self.surfaces
                                .as_ref()
                                .map_or("?", |s| s.ssh.status.as_str())
                        ));
                        self.note("SSH fetched");
                    }
                    FetchEvent::Cli(cli) => {
                        self.box_cli = BoxCliView::Known(cli.status.clone());
                        self.cli_current = cli.current;
                        self.ensure_surfaces_shell();
                        if let Some(s) = self.surfaces.as_mut() {
                            s.cli = cli;
                        }
                        self.push_log(format!("fetch cli={}", self.box_cli.as_label()));
                        self.note("CLI fetched");
                    }
                    FetchEvent::Api(api) => {
                        self.ensure_surfaces_shell();
                        self.push_log(format!("fetch api health={}", api.health));
                        if let Some(s) = self.surfaces.as_mut() {
                            s.api = api;
                        }
                        self.note("API fetched");
                    }
                    FetchEvent::Mcp(pc, bx) => {
                        self.ensure_surfaces_shell();
                        self.push_log(format!(
                            "fetch mcp pc={} box={}",
                            pc.http_reach, bx.http_reach
                        ));
                        if let Some(s) = self.surfaces.as_mut() {
                            s.mcp_pc = pc;
                            s.mcp_box = bx;
                        }
                        self.note("MCP fetched");
                    }
                    FetchEvent::Failed(e) => {
                        self.push_log(format!("ERROR fetch: {e}"));
                        self.note(format!("Fetch failed: {e}"));
                    }
                }
                self.refresh_panel_text();
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.fetch_inflight = false;
                self.fetch_rx = None;
            }
        }
    }
    /// Accept the current confirm modal (run step, install, reboot, save token).
    pub fn resolve_confirm_yes(&mut self) {
        let Some(Modal::Confirm(kind)) = self.modal.take() else {
            return;
        };
        match kind {
            ConfirmKind::DestructiveStep(id) => self.execute_step(&id),
            ConfirmKind::Reboot | ConfirmKind::RebootAfterApply => {
                if !self.apply {
                    self.note("PLAN: reboot not sent (Tab → APPLY)");
                    self.push_log("PLAN: reboot not sent (press Tab for APPLY)");
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
            ConfirmKind::InstallStatusApi { remote } => {
                self.modal = Some(Modal::Confirm(ConfirmKind::InstallMcp {
                    remote,
                    status_api: true,
                }));
                self.message = "Install MCP? Enter/y confirm, Esc/n cancel.".into();
            }
            ConfirmKind::InstallMcp { remote, status_api } => {
                self.run_full_pipeline_with_ecosystem(
                    remote,
                    EcosystemInstallChoice {
                        status_api,
                        mcp: true,
                    },
                );
            }
        }
    }
    /// Decline the current confirm modal (skip install bits or cancel).
    pub fn resolve_confirm_no(&mut self) {
        let Some(Modal::Confirm(kind)) = self.modal.take() else {
            self.modal = None;
            self.message = "Cancelled".into();
            return;
        };
        match kind {
            ConfirmKind::InstallStatusApi { remote } => {
                self.modal = Some(Modal::Confirm(ConfirmKind::InstallMcp {
                    remote,
                    status_api: false,
                }));
                self.message = "Install MCP? Enter/y confirm, Esc/n cancel.".into();
            }
            ConfirmKind::InstallMcp { remote, status_api } => {
                self.run_full_pipeline_with_ecosystem(
                    remote,
                    EcosystemInstallChoice {
                        status_api,
                        mcp: false,
                    },
                );
            }
            ConfirmKind::SaveToken(_) => {
                self.message = "Cancelled".into();
                self.push_log("Skipped saving status-api bearer locally");
                self.maybe_offer_reboot_after_token();
            }
            _ => {
                self.message = "Cancelled".into();
            }
        }
    }
    /// After token save, offer reboot when a remote apply installed payload.
    pub fn maybe_offer_reboot_after_token(&mut self) {
        if !self.pending_reboot_offer {
            return;
        }
        self.pending_reboot_offer = false;
        self.modal = Some(Modal::Confirm(ConfirmKind::RebootAfterApply));
        self.message = "Reboot after apply? Enter/y confirm, Esc/n cancel.".into();
    }
    /// Upload / sync the CLI agent to the box (`s0`).
    pub fn run_s0_sync(&mut self) {
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
    /// Refresh setup list, overview text, and embedded surface probes.
    pub fn refresh_local(&mut self) {
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
        let _ = writeln!(overview, "Hostname: {}", box_st.hostname);
        let _ = writeln!(
            overview,
            "Root: {}  Docker: {}  Full env: {}  Minimal env: {}",
            box_st.doctor.is_root,
            box_st.doctor.docker_present,
            box_st.doctor.full_env,
            box_st.doctor.minimal_env
        );
        for n in &box_st.doctor.notes {
            let _ = writeln!(overview, "- {n}");
        }
        overview.push_str("\nContainers:\n");
        if box_st.containers.is_empty() {
            overview.push_str("  (none)\n");
        } else {
            for c in &box_st.containers {
                let _ = writeln!(overview, "  {} {}", c.names, c.status);
            }
        }
        overview.push_str("\nURLs:\n");
        for u in &box_st.urls {
            let mark = if u.up { "up" } else { "down" };
            let _ = writeln!(overview, "  {} [{}]: {}", u.name, mark, u.url);
        }
        let _ = writeln!(overview, "\nLeases: {}", box_st.leases.len());
        for l in box_st.leases.iter().take(12) {
            let _ = writeln!(overview, "  {} {}", l.hostname, l.ip);
        }
        overview.push_str("\nBackup:\n");
        let _ = writeln!(
            overview,
            "  initial_setup: {}",
            box_st.backup.initial_setup_present
        );
        if box_st.backup.timestamped.is_empty() {
            overview.push_str("  timestamped: (none)\n");
        } else {
            let recent: Vec<_> = box_st.backup.timestamped.iter().rev().take(5).collect();
            let _ = writeln!(
                overview,
                "  timestamped ({}): {}",
                box_st.backup.timestamped.len(),
                recent
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let _ = writeln!(
            overview,
            "  disk root={} safe={} blockers={}",
            box_st.backup.disk.root_source,
            box_st.backup.disk.safe_to_apply,
            box_st.backup.disk.blockers.len()
        );
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
    /// Run argv on the box via the remote runner; handle token/reboot prompts.
    pub fn run_remote_cli(
        &mut self,
        rest: &[&str],
        use_sudo: bool,
        ecosystem: EcosystemInstallChoice,
    ) {
        let Some(options) = self.remote_opts() else {
            return;
        };
        let mut cli_args = Vec::new();
        if self.apply {
            cli_args.push("--apply".into());
        }
        if self.skip_piper {
            cli_args.push("--skip-piper".into());
        }
        let stacks_csv = self.stack_opts.to_csv();
        if !stacks_csv.is_empty() {
            cli_args.push(format!("--stacks={stacks_csv}"));
        }
        if ecosystem.status_api {
            cli_args.push("--install-status-api".into());
        } else {
            cli_args.push("--no-install-status-api".into());
        }
        if ecosystem.mcp {
            cli_args.push("--install-mcp".into());
        } else {
            cli_args.push("--no-install-mcp".into());
        }
        for a in rest {
            cli_args.push((*a).to_owned());
        }
        match remote_run_cli(
            &SystemProcessRunner,
            &RemoteRunRequest {
                options,
                cli_args,
                flags: RemoteRunFlags {
                    use_sudo,
                    install_payload_on_success: ecosystem.any(),
                    offer_reboot_on_success: false,
                    capture_output: false,
                    ..Default::default()
                },
                ecosystem,
            },
        ) {
            Ok(outcome) => {
                for line in outcome.log.lines() {
                    self.push_log(line.to_owned());
                }
                let offer_reboot = ecosystem.any() && self.apply;
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
    /// Build a [`HostContext`] from apply mode, pipeline kind, and skip-piper.
    pub fn make_ctx(&self) -> HostContext {
        let mode = if self.apply {
            ApplyMode::Apply
        } else {
            ApplyMode::DryRun
        };
        let mut ctx = HostContext::new(mode, self.kind).with_prompts(Box::new(StdioPrompts));
        ctx.skip_piper = self.skip_piper;
        ctx.stack_opts = self.stack_opts;
        ctx.ecosystem = EcosystemInstallChoice::both();
        ctx
    }
    /// Step id of the highlighted setup-list row, when any.
    pub fn selected_step_id(&self) -> Option<String> {
        let idx = self.step_state.selected()?;
        self.status_lines
            .get(idx)
            .and_then(|l| l.split(" | ").next())
            .map(str::to_string)
    }
    /// Append a timestamped log line (keeps the last 500).
    pub fn push_log(&mut self, line: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.logs.push(format!("{ts} {}", line.into()));
        if self.logs.len() > 500 {
            self.logs.drain(0..self.logs.len() - 500);
        }
    }
    /// Clear the in-memory log buffer.
    pub fn clear_logs(&mut self) {
        self.logs.clear();
        self.message = "Logs cleared".into();
    }
    /// Move to the next tab for the current local/remote mode.
    pub fn next_screen(&mut self) {
        self.screen = self.screen.next(self.is_remote());
        self.refresh_panel_text();
    }
    /// Move to the previous tab for the current local/remote mode.
    pub fn prev_screen(&mut self) {
        self.screen = self.screen.prev(self.is_remote());
        self.refresh_panel_text();
    }
    /// Jump to a tab and close help.
    pub fn select_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.help_open = false;
        self.refresh_panel_text();
    }
    /// Run the highlighted setup step (confirm when destructive + apply).
    pub fn run_selected(&mut self) {
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
                BoxCliView::Known(_) => "Run s0 (Sync CLI) before other steps".into(),
            };
            return;
        }
        let ctx_probe = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx_probe, self.kind);
        if let Some(row) = report.steps.iter().find(|s| s.id == id) {
            if row.destructive && self.apply {
                self.modal = Some(Modal::Confirm(ConfirmKind::DestructiveStep(id)));
                self.message = "Step is destructive. Enter/y confirm, Esc/n cancel.".into();
                return;
            }
        }
        self.execute_step(&id);
    }
    /// Execute one setup step locally or remotely, then refresh.
    pub fn execute_step(&mut self, id: &str) {
        self.push_log(format!("Running step {id} (apply={})", self.apply));
        if self.remote.is_some() {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(
                &["setup", "step", id, kind],
                self.apply,
                EcosystemInstallChoice::none(),
            );
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
    /// Run the full pipeline; prompt for ecosystem installs when applying full.
    pub fn run_all(&mut self) {
        if self.remote.is_some() && !self.cli_current {
            self.message = match &self.box_cli {
                BoxCliView::Probing => "Still probing box CLI...".into(),
                BoxCliView::Known(_) => "Run s0 (Sync CLI) before running all steps".into(),
            };
            return;
        }
        if self.apply && self.kind == SetupKind::Full {
            let remote = self.remote.is_some();
            self.modal = Some(Modal::Confirm(ConfirmKind::InstallStatusApi { remote }));
            self.message = "Install status-api? Enter/y yes (default), Esc/n skip.".into();
            return;
        }
        self.run_full_pipeline_with_ecosystem(
            self.remote.is_some(),
            EcosystemInstallChoice::none(),
        );
    }
    /// Run all pipeline steps with the chosen ecosystem install set.
    pub fn run_full_pipeline_with_ecosystem(
        &mut self,
        remote: bool,
        ecosystem: EcosystemInstallChoice,
    ) {
        self.push_log(format!(
            "Running all pipeline steps (apply={}, status_api={}, mcp={})",
            self.apply, ecosystem.status_api, ecosystem.mcp
        ));
        if remote {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(&["setup", "run", kind], self.apply, ecosystem);
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
                if self.apply && self.kind == SetupKind::Full && ecosystem.any() {
                    match install_ecosystem_after_embedded_apply(
                        &SystemProcessRunner,
                        std::path::Path::new(DEFAULT_INSTALL_DIR),
                        ecosystem,
                    ) {
                        Ok(token) => {
                            self.push_log(format!(
                                "Installed selected ecosystem services under {DEFAULT_INSTALL_DIR}"
                            ));
                            if let Some(hex) = token {
                                self.modal = Some(Modal::Confirm(ConfirmKind::SaveToken(hex)));
                                self.message =
                                    "Save API token? Enter/y confirm, Esc/n cancel.".into();
                            }
                        }
                        Err(e) => {
                            self.push_log(format!("ERROR ecosystem install: {e}"));
                            self.message = format!("Ecosystem install failed: {e}");
                        }
                    }
                } else if self.apply && self.kind == SetupKind::Full && !ecosystem.any() {
                    self.push_log("Skipped status-api / MCP install");
                }
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Pipeline failed: {e}");
            }
        }
        self.refresh();
    }
    /// Timestamped `/etc` backup on the local host (apply gate applies).
    pub fn run_backup_etc(&mut self) {
        self.push_log(format!("Timestamped /etc backup (apply={})", self.apply));
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
    /// Probe disk-backup readiness and switch to the Logs tab.
    pub fn show_disk_backup_status(&mut self) {
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

/// After reboot is accepted: wait until the box stops answering once, then until it replies again.
/// Esc sets `cancel`. Returns `Err("cancelled")` or timeout.
fn wait_until_box_replies(host: &str, cancel: &AtomicBool) -> std::result::Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut saw_down = false;
    let mut interval = Duration::from_millis(500);
    let max_interval = Duration::from_secs(3);

    while Instant::now() < deadline {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let up = ssh_host_up(host);
        if !saw_down {
            if !up {
                saw_down = true;
            }
        } else if up {
            return Ok(());
        }
        sleep_cancellable(interval, cancel)?;
        interval = (interval + Duration::from_millis(250)).min(max_interval);
    }
    Err("timed out waiting for box to reply".into())
}

fn ssh_host_up(host: &str) -> bool {
    std::process::Command::new("ssh")
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
        .status()
        .is_ok_and(|s| s.success())
}

fn sleep_cancellable(total: Duration, cancel: &AtomicBool) -> std::result::Result<(), String> {
    let mut slept = Duration::ZERO;
    let slice = Duration::from_millis(100);
    while slept < total {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        thread::sleep(slice.min(total.checked_sub(slept).unwrap()));
        slept += slice;
    }
    Ok(())
}
