//! Ops KPI viewer for the Horto status API (GPUI). View-only: numeric tiles only.

mod kpis;

use anyhow::Result;
use clap::Parser;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context as GpuiContext, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use kpis::{derive_kpis, BoxStatus, Health, KpiTile, KpiTone};
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

fn tone_border(tone: KpiTone) -> u32 {
    match tone {
        KpiTone::Ok => 0x3d8b5a,
        KpiTone::Warn => 0xc9a227,
        KpiTone::Bad => 0xc44b4b,
        KpiTone::Neutral => 0x555555,
    }
}

fn kpi_card(tile: KpiTile) -> impl IntoElement {
    let border = tone_border(tile.tone);
    div()
        .w(px(200.0))
        .min_h(px(100.0))
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(0x252526))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(rgb(0xaaaaaa))
                .child(SharedString::from(tile.label)),
        )
        .child(
            div()
                .text_3xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(SharedString::from(tile.value)),
        )
}

impl Render for SoftClient {
    fn render(&mut self, _window: &mut Window, cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let tiles = derive_kpis(self.snap.health_ok, self.snap.status.as_ref());
        let hostname = self
            .snap
            .status
            .as_ref()
            .map(|s| s.hostname.as_str())
            .filter(|h| !h.is_empty())
            .unwrap_or("unknown");
        let header = SharedString::from(format!("{hostname} · {}", self.snap.base_url));
        let err = self.snap.error.clone().map(SharedString::from);

        div()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xf0f0f0))
            .size_full()
            .p_6()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Horto KPIs"),
            )
            .child(div().text_sm().text_color(rgb(0x999999)).child(header))
            .when_some(err, |this, e| {
                this.child(div().text_color(rgb(0xffcc66)).child(e))
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .children(tiles.into_iter().map(kpi_card)),
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
                    .w(px(120.0))
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
    eprintln!("horto-os-ui-kpi: opening window for {} …", snap.base_url);

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(880.0), px(640.0)), cx);
        let open = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Horto KPIs".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                focus: true,
                show: true,
                app_id: Some("network.hortos.os-ui-kpi".into()),
                window_min_size: Some(size(px(640.0), px(480.0))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Horto KPIs");
                cx.new(|_| SoftClient {
                    snap: snap.clone(),
                    token: token.clone(),
                })
            },
        );
        match open {
            Ok(_) => cx.activate(true),
            Err(e) => {
                eprintln!("horto-os-ui-kpi: failed to open window: {e:#}");
                cx.quit();
            }
        }
    });
    Ok(())
}
