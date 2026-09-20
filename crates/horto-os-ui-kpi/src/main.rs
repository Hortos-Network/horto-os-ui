//! Horto KPI board (GPUI): equal 3x3 live charts against the status API and optional EVCC.

mod charts;
mod config;
mod demo;
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
use history::LiveHistory;
use horto_os_ui_shared::init_tracing;
use kpis::{evcc_base_url, sample_metrics, BoxStatus, Health};
use std::time::Duration;
use tracing::{error, info};

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
    about = "Horto KPI board (GPUI live charts)",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    /// Base URL of horto-os-ui-status-api
    #[arg(
        long,
        env = "HORTO_STATUS_API_URL",
        default_value = "http://localhost:8787"
    )]
    url: String,
    #[arg(long, env = "HORTO_API_TOKEN")]
    token: Option<String>,
    /// Override EVCC base URL (default: EVCC link from `/v1/status`)
    #[arg(long, env = "HORTO_EVCC_URL")]
    evcc_url: Option<String>,
    /// Synthetic animated series (default on until live EVCC is wired)
    #[arg(long, env = "HORTO_KPI_DEMO", default_value_t = true, action = clap::ArgAction::Set)]
    demo: bool,
    /// History depth (samples) for line charts
    #[arg(long, env = "HORTO_KPI_HISTORY", default_value_t = 60)]
    history: usize,
    /// Poll interval seconds
    #[arg(long, env = "HORTO_KPI_POLL_SECS", default_value_t = 1)]
    poll_secs: u64,
}

#[derive(Clone)]
struct Snapshot {
    base_url: String,
    health_ok: Option<bool>,
    status: Option<BoxStatus>,
    error: Option<String>,
    evcc_note: Option<String>,
    energy: history::MetricSample,
}

fn fetch_snapshot(base_url: &str, token: Option<&str>, evcc_override: Option<&str>) -> Snapshot {
    let mut snap = Snapshot {
        base_url: base_url.to_string(),
        health_ok: None,
        status: None,
        error: None,
        evcc_note: None,
        energy: history::MetricSample::default(),
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
    let evcc_base = evcc_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| evcc_base_url(snap.status.as_ref()));
    match evcc_base {
        Some(url) => match evcc::fetch_powers(&url) {
            Ok(p) => {
                pv = p.pv_w;
                grid = p.grid_w;
                home = p.home_w;
                charge = p.charge_w;
                if pv.is_none() && grid.is_none() && home.is_none() {
                    snap.evcc_note = Some(format!("EVCC {url}/api/state: no power fields"));
                }
            }
            Err(e) => snap.evcc_note = Some(e),
        },
        None => {
            snap.evcc_note =
                Some("No EVCC URL (set HORTO_EVCC_URL or add an EVCC service link)".into());
        }
    }
    snap.energy = sample_metrics(snap.status.as_ref(), pv, grid, home, charge);
    snap
}

struct SoftClient {
    snap: Snapshot,
    token: Option<String>,
    evcc_url: Option<String>,
    demo: bool,
    tick: u32,
    history: LiveHistory,
}

impl SoftClient {
    fn refresh(&mut self) {
        if self.demo {
            self.tick = self.tick.wrapping_add(1);
            let sample = demo::sample_at(self.tick);
            self.snap.energy = sample.clone();
            self.snap.health_ok = Some(true);
            self.snap.error = None;
            self.snap.evcc_note = None;
            self.history.push_sample(&sample);
            return;
        }
        self.snap = fetch_snapshot(
            &self.snap.base_url.clone(),
            self.token.as_deref(),
            self.evcc_url.as_deref(),
        );
        self.history.push_sample(&self.snap.energy);
    }
}

impl Render for SoftClient {
    fn render(&mut self, _window: &mut Window, _cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let hostname = if self.demo {
            "demo-box"
        } else {
            self.snap
                .status
                .as_ref()
                .map(|s| s.hostname.as_str())
                .filter(|h| !h.is_empty())
                .unwrap_or("unknown")
        };
        let api = if self.demo {
            "demo data"
        } else {
            match self.snap.health_ok {
                Some(true) => "API online",
                Some(false) => "API down",
                None => "API unknown",
            }
        };
        let subtitle = SharedString::from(format!(
            "{hostname}  |  {}  |  {api}  |  v{VERSION} ({GIT_COMMIT})",
            self.snap.base_url
        ));
        let err = if self.demo {
            None
        } else {
            self.snap.error.clone().map(SharedString::from)
        };
        let energy_note = if self.demo {
            None
        } else if self.history.pv_w.points.is_empty()
            && self.history.grid_w.points.is_empty()
            && self.history.home_w.points.is_empty()
            && self.history.charge_w.points.is_empty()
        {
            self.snap.evcc_note.as_deref()
        } else {
            None
        };

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
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x9fd89f))
                            .child("Horto KPI"),
                    )
                    .child(div().text_xs().text_color(rgb(0x6a8a6a)).child(subtitle)),
            )
            .when_some(err, |this, e| {
                this.child(div().text_sm().text_color(rgb(0xe0b040)).child(e))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3()
                            .w_full()
                            .flex_1()
                            .child(charts::metric_tile(
                                "PV",
                                "W",
                                &self.history.pv_w.points,
                                0x5ecf6b,
                                energy_note,
                            ))
                            .child(charts::metric_tile(
                                "Grid",
                                "W",
                                &self.history.grid_w.points,
                                0xe0b040,
                                energy_note,
                            ))
                            .child(charts::metric_tile(
                                "Home",
                                "W",
                                &self.history.home_w.points,
                                0x5aa8e0,
                                energy_note,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3()
                            .w_full()
                            .flex_1()
                            .child(charts::metric_tile(
                                "Charge",
                                "W",
                                &self.history.charge_w.points,
                                0xc070e0,
                                energy_note,
                            ))
                            .child(charts::metric_tile(
                                "Containers",
                                "up",
                                &self.history.containers_up.points,
                                0x5ecf6b,
                                None,
                            ))
                            .child(charts::metric_tile(
                                "Services",
                                "up",
                                &self.history.services_up.points,
                                0x5aa8e0,
                                None,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3()
                            .w_full()
                            .flex_1()
                            .child(charts::metric_tile(
                                "Setup",
                                "%",
                                &self.history.setup_pct.points,
                                0x5ecf6b,
                                None,
                            ))
                            .child(charts::metric_tile(
                                "Doctor",
                                "%",
                                &self.history.doctor_pct.points,
                                0x5aa8e0,
                                None,
                            ))
                            .child(charts::metric_tile(
                                "Leases",
                                "DHCP",
                                &self.history.leases.points,
                                0xe0b040,
                                None,
                            )),
                    ),
            )
    }
}

fn main() -> Result<()> {
    init_tracing("info");
    let cli = Cli::parse();
    let cfg = DashboardConfig {
        history: cli.history.max(8),
        poll_secs: cli.poll_secs.max(1),
    };
    let token = cli.token.clone();
    let evcc_url = cli.evcc_url.clone();
    let demo = cli.demo;
    let tick = 60_u32;
    let mut history = LiveHistory::with_capacity(cfg.history);
    let snap = if demo {
        demo::seed_history(&mut history, tick, cfg.history);
        let energy = demo::sample_at(tick);
        Snapshot {
            base_url: cli.url.clone(),
            health_ok: Some(true),
            status: None,
            error: None,
            evcc_note: None,
            energy,
        }
    } else {
        let snap = fetch_snapshot(&cli.url, token.as_deref(), evcc_url.as_deref());
        history.push_sample(&snap.energy);
        snap
    };
    if demo {
        info!(
            "horto-os-ui-kpi: DEMO board (synthetic series, poll {}s). Use --demo false for live API.",
            cfg.poll_secs
        );
    } else {
        info!(
            "horto-os-ui-kpi: board for {} (poll {}s) …",
            snap.base_url, cfg.poll_secs
        );
        if let Some(n) = &snap.evcc_note {
            info!("horto-os-ui-kpi: {n}");
        }
    }

    let poll = Duration::from_secs(cfg.poll_secs);
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(860.0)), cx);
        let open = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(if demo {
                        "Horto KPI (demo)".into()
                    } else {
                        "Horto KPI".into()
                    }),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                focus: true,
                show: true,
                app_id: Some("network.hortos.os-ui-kpi".into()),
                window_min_size: Some(size(px(960.0), px(720.0))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(if demo {
                    "Horto KPI (demo)"
                } else {
                    "Horto KPI"
                });
                let entity = cx.new(|_| SoftClient {
                    snap: snap.clone(),
                    token: token.clone(),
                    evcc_url: evcc_url.clone(),
                    demo,
                    tick,
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
                error!("horto-os-ui-kpi: failed to open window: {e:#}");
                cx.quit();
            }
        }
    });
    Ok(())
}
