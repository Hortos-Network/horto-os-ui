//! Ops KPI / status viewer for the Horto status API (GPUI). View-only: no remote install.

use anyhow::{Context, Result};
use clap::Parser;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context as GpuiContext, SharedString,
    Window, WindowBounds, WindowOptions,
};
use serde::Deserialize;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_COMMIT: &str = env!("GIT_COMMIT_HASH");
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_COMMIT_HASH"),
    ")"
);

fn footer_line() -> String {
    format!("v{VERSION} · {GIT_COMMIT}")
}

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-kpi",
    about = "Horto OS UI ops KPI viewer (GPUI)",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    /// Base URL of horto-os-ui-status-api on the box (example: http://192.168.1.10:8787)
    #[arg(long, env = "HORTO_BOX_URL", default_value = "http://localhost:8787")]
    url: String,
    #[arg(long, env = "HORTO_API_TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct Health {
    ok: bool,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ContainerInfo {
    names: String,
    image: String,
    status: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct UrlInfo {
    name: String,
    url: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct BackupStatus {
    initial_setup_present: bool,
    #[serde(default)]
    timestamped: Vec<String>,
    #[serde(default)]
    disk: DiskBackupProbe,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct DiskBackupProbe {
    #[serde(default)]
    root_source: String,
    #[serde(default)]
    safe_to_apply: bool,
    #[serde(default)]
    blockers: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct BoxStatus {
    hostname: String,
    containers: Vec<ContainerInfo>,
    urls: Vec<UrlInfo>,
    #[serde(default)]
    backup: BackupStatus,
}

#[derive(Clone)]
struct Snapshot {
    base_url: String,
    health_ok: Option<bool>,
    status: Option<BoxStatus>,
    error: Option<String>,
}

fn fetch_snapshot(base_url: &str, token: Option<&str>) -> Snapshot {
    let mut snap = Snapshot {
        base_url: base_url.to_string(),
        health_ok: None,
        status: None,
        error: None,
    };
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            snap.error = Some(e.to_string());
            return snap;
        }
    };
    let health_url = format!("{}/health", base_url.trim_end_matches('/'));
    match client
        .get(&health_url)
        .send()
        .and_then(|r| r.json::<Health>())
    {
        Ok(h) => snap.health_ok = Some(h.ok),
        Err(e) => {
            snap.health_ok = Some(false);
            snap.error = Some(format!("health: {e}"));
        }
    }
    let mut req = client.get(format!("{}/v1/status", base_url.trim_end_matches('/')));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    match req
        .send()
        .and_then(|r| r.error_for_status()?.json::<BoxStatus>())
    {
        Ok(s) => snap.status = Some(s),
        Err(e) => snap.error = Some(format!("status: {e}")),
    }
    snap
}

struct SoftClient {
    snap: Snapshot,
    token: Option<String>,
}

impl SoftClient {
    fn refresh(&mut self) {
        self.snap = fetch_snapshot(&self.snap.base_url.clone(), self.token.as_deref());
    }
}

impl Render for SoftClient {
    fn render(&mut self, _window: &mut Window, cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let health = match self.snap.health_ok {
            Some(true) => SharedString::from("API health: ok"),
            Some(false) => SharedString::from("API health: down"),
            None => SharedString::from("API health: unknown"),
        };
        let hostname = self
            .snap
            .status
            .as_ref()
            .map(|s| format!("Hostname: {}", s.hostname))
            .unwrap_or_else(|| "Hostname: (unavailable)".into());
        let mut container_lines: Vec<SharedString> = Vec::new();
        if let Some(ref st) = self.snap.status {
            if st.containers.is_empty() {
                container_lines.push("(none)".into());
            } else {
                for c in &st.containers {
                    container_lines.push(format!("{}  {}  {}", c.names, c.image, c.status).into());
                }
            }
        }
        let mut link_lines: Vec<SharedString> = Vec::new();
        if let Some(ref st) = self.snap.status {
            for u in &st.urls {
                link_lines.push(format!("{}: {}", u.name, u.url).into());
            }
        }
        let mut backup_lines: Vec<SharedString> = Vec::new();
        if let Some(ref st) = self.snap.status {
            backup_lines.push(
                format!(
                    "initial_setup: {}  timestamped: {}",
                    st.backup.initial_setup_present,
                    st.backup.timestamped.len()
                )
                .into(),
            );
            backup_lines.push(
                format!(
                    "disk root={} safe={} blockers={}",
                    st.backup.disk.root_source,
                    st.backup.disk.safe_to_apply,
                    st.backup.disk.blockers.len()
                )
                .into(),
            );
        }
        let err = self.snap.error.clone().map(SharedString::from);
        let box_url = SharedString::from(format!("Box: {}", self.snap.base_url));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xf0f0f0))
            .size_full()
            .p_6()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Horto OS UI ops KPI"),
            )
            .child(div().child(box_url))
            .child(div().child(health))
            .when_some(err, |this, e| {
                this.child(div().text_color(rgb(0xffcc66)).child(e))
            })
            .child(div().child(SharedString::from(hostname)))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Containers"),
            )
            .children(container_lines.into_iter().map(|line| div().child(line)))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Links"))
            .children(link_lines.into_iter().map(|line| div().child(line)))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Backup"),
            )
            .children(backup_lines.into_iter().map(|line| div().child(line)))
            .child(
                div().text_sm().text_color(rgb(0xaaaaaa)).child(
                    "View-only. Install, backup apply, and network stay on the box (CLI/TUI).",
                ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x888888))
                    .child(SharedString::from(footer_line())),
            )
            .child(
                div()
                    .id("refresh")
                    .px_3()
                    .py_2()
                    .bg(rgb(0x3a6ea5))
                    .rounded_md()
                    .cursor_pointer()
                    .child("Refresh")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh();
                        cx.notify();
                    })),
            )
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = cli.token.clone();
    let snap = fetch_snapshot(&cli.url, token.as_deref());

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| SoftClient {
                    snap: snap.clone(),
                    token: token.clone(),
                })
            },
        )
        .context("open window")
        .expect("open horto-os-ui-kpi window");
        cx.activate(true);
    });
    Ok(())
}
