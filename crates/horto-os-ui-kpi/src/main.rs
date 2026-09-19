//! Control-room KPI dashboard (GPUI): live charts for Horto box ops and energy.

mod charts;
mod config;
mod evcc;
mod history;
mod kpis;

use anyhow::Result;
use clap::Parser;
use config::DashboardConfig;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context as GpuiContext, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use history::{LiveHistory, MetricSample};
use kpis::{evcc_base_url, sample_metrics, BoxStatus, Health};
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_COMMIT: &str = env!("GIT_COMMIT_HASH");
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_COMMIT_HASH"),
    ")"
);

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-kpi",
    about = "Horto control-room KPI dashboard (GPUI charts)",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    /// Base URL of horto-os-ui-status-api on the box
    #[arg(long, env = "HORTO_BOX_URL", default_value = "http://localhost:8787")]
    url: String,
    #[arg(long, env = "HORTO_API_TOKEN")]
    token: Option<String>,
    /// Comma list: energy,fleet,readiness,network (default: all)
    #[arg(long, env = "HORTO_KPI_PANELS", default_value = "")]
    panels: String,
    /// History depth (samples) for line charts
    #[arg(long, env = "HORTO_KPI_HISTORY", default_value_t = 60)]
    history: usize,
    /// Poll interval seconds
    #[arg(long, env = "HORTO_KPI_POLL_SECS", default_value_t = 2)]
    poll_secs: u64,
}

#[derive(Clone)]
struct Snapshot {
    base_url: String,
    health_ok: Option<bool>,
    status: Option<BoxStatus>,
    error: Option<String>,
    energy: MetricSample,
}

fn fetch_snapshot(base_url: &str, token: Option<&str>) -> Snapshot {
    let mut snap = Snapshot {
        base_url: base_url.to_string(),
        health_ok: None,
        status: None,
        error: None,
        energy: MetricSample::default(),
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

    let mut pv = None;
    let mut grid = None;
    let mut home = None;
    let mut charge = None;
    if let Some(evcc_url) = evcc_base_url(snap.status.as_ref()) {
        let p = evcc::fetch_powers(&evcc_url);
        pv = p.pv_w;
        grid = p.grid_w;
        home = p.home_w;
        charge = p.charge_w;
    }
    snap.energy = sample_metrics(snap.status.as_ref(), pv, grid, home, charge);
    snap
}

struct SoftClient {
    snap: Snapshot,
    token: Option<String>,
    cfg: DashboardConfig,
    history: LiveHistory,
}

impl SoftClient {
    fn refresh(&mut self) {
        self.snap = fetch_snapshot(&self.snap.base_url.clone(), self.token.as_deref());
        self.history.push_sample(&self.snap.energy);
    }
}

impl Render for SoftClient {
    fn render(&mut self, _window: &mut Window, _cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let hostname = self
            .snap
            .status
            .as_ref()
            .map(|s| s.hostname.as_str())
            .filter(|h| !h.is_empty())
            .unwrap_or("unknown");
        let api = match self.snap.health_ok {
            Some(true) => "API online",
            Some(false) => "API down",
            None => "API unknown",
        };
        let header = SharedString::from(format!(
            "{hostname}  ·  {}  ·  {api}  ·  v{VERSION} · {GIT_COMMIT}",
            self.snap.base_url
        ));
        let err = self.snap.error.clone().map(SharedString::from);

        let mut rows = div().flex().flex_col().gap_3().w_full().flex_1();

        if self.cfg.energy {
            let energy_series: Vec<(&str, &[f32], u32)> = [
                ("PV", self.history.pv_w.points.as_slice(), 0x5ecf6b_u32),
                ("Grid", self.history.grid_w.points.as_slice(), 0xe0b040_u32),
                ("Home", self.history.home_w.points.as_slice(), 0x5aa8e0_u32),
                (
                    "Charge",
                    self.history.charge_w.points.as_slice(),
                    0xc070e0_u32,
                ),
            ]
            .into_iter()
            .filter(|(_, vals, _)| vals.len() >= 2)
            .collect();
            if energy_series.is_empty() {
                rows = rows.child(placeholder_panel(
                    "Energy (EVCC)",
                    "Waiting for EVCC /api/state samples (service link EVCC + live power).",
                ));
            } else {
                rows = rows.child(charts::line_panel("Energy", &energy_series, "W"));
            }
        }

        if self.cfg.fleet {
            let fleet = [
                (
                    "Containers up",
                    self.history.containers_up.points.as_slice(),
                    0x5ecf6b_u32,
                ),
                (
                    "Services up",
                    self.history.services_up.points.as_slice(),
                    0x5aa8e0_u32,
                ),
            ];
            rows = rows.child(charts::line_panel("Fleet", &fleet, "count"));
        }

        let mut bottom = div().flex().flex_row().gap_3().w_full();
        if self.cfg.readiness {
            let setup = *self.history.setup_pct.points.last().unwrap_or(&0.0);
            let doctor = *self.history.doctor_pct.points.last().unwrap_or(&0.0);
            let cont_t = self.snap.energy.containers_total.max(1.0);
            let svc_t = self.snap.energy.services_total.max(1.0);
            let cont_pct = 100.0 * self.snap.energy.containers_up / cont_t;
            let svc_pct = 100.0 * self.snap.energy.services_up / svc_t;
            bottom = bottom.child(charts::bar_panel(
                "Readiness",
                &[
                    ("Setup done", setup, 0x5ecf6b),
                    ("Doctor checks", doctor, 0x5aa8e0),
                    ("Containers up", cont_pct, 0xe0b040),
                    ("Services up", svc_pct, 0xc070e0),
                ],
            ));
        }
        if self.cfg.network {
            let net = [(
                "DHCP leases",
                self.history.leases.points.as_slice(),
                0x5aa8e0_u32,
            )];
            bottom = bottom.child(charts::line_panel("Network", &net, "leases"));
        }
        rows = rows.child(bottom);

        div()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(0x0a0f0a))
            .text_color(rgb(0xe8f0e8))
            .size_full()
            .p_5()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x9fd89f))
                    .child("Horto Control Room"),
            )
            .child(div().text_xs().text_color(rgb(0x6a8a6a)).child(header))
            .when_some(err, |this, e| {
                this.child(div().text_color(rgb(0xe0b040)).child(e))
            })
            .child(rows)
    }
}

fn placeholder_panel(title: &str, body: &str) -> impl IntoElement {
    let title = SharedString::from(title.to_owned());
    let body = SharedString::from(body.to_owned());
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x2a3a2a))
        .bg(rgb(0x121812))
        .min_h(px(180.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0xb8d4b8))
                .child(title),
        )
        .child(div().text_sm().text_color(rgb(0x6a8a6a)).child(body))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = DashboardConfig::from_panels_csv(&cli.panels);
    cfg.history = cli.history.max(8);
    cfg.poll_secs = cli.poll_secs.max(1);
    let token = cli.token.clone();
    let snap = fetch_snapshot(&cli.url, token.as_deref());
    let mut history = LiveHistory::with_capacity(cfg.history);
    history.push_sample(&snap.energy);
    eprintln!(
        "horto-os-ui-kpi: control room for {} (poll {}s) …",
        snap.base_url, cfg.poll_secs
    );

    let poll = Duration::from_secs(cfg.poll_secs);
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
        let open = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Horto Control Room".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                focus: true,
                show: true,
                app_id: Some("network.hortos.os-ui-kpi".into()),
                window_min_size: Some(size(px(960.0), px(640.0))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Horto Control Room");
                let entity = cx.new(|_| SoftClient {
                    snap: snap.clone(),
                    token: token.clone(),
                    cfg: cfg.clone(),
                    history,
                });
                cx.spawn({
                    let entity = entity.downgrade();
                    async move |cx| loop {
                        cx.background_executor().timer(poll).await;
                        let ok = entity
                            .update(cx, |this, cx| {
                                this.refresh();
                                cx.notify();
                            })
                            .is_ok();
                        if !ok {
                            break;
                        }
                    }
                })
                .detach();
                entity
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
