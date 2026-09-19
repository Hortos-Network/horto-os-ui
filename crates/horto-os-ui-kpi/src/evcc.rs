//! Optional EVCC `/api/state` scrape for live energy watts.

use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Default, Clone)]
pub struct EvccPowers {
    pub pv_w: Option<f32>,
    pub grid_w: Option<f32>,
    pub home_w: Option<f32>,
    pub charge_w: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct EvccState {
    #[serde(default)]
    grid: EvccNode,
    #[serde(default)]
    pv: EvccNode,
    #[serde(default)]
    #[serde(alias = "homePower")]
    home_power: Option<f32>,
    #[serde(default)]
    loadpoints: Vec<EvccLoadpoint>,
}

#[derive(Debug, Default, Deserialize)]
struct EvccNode {
    #[serde(default)]
    power: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct EvccLoadpoint {
    #[serde(default)]
    charge_power: Option<f32>,
    #[serde(default)]
    #[serde(alias = "chargePower")]
    charge_power_camel: Option<f32>,
}

/// GET `{base}/api/state` and map common EVCC power fields.
#[must_use]
pub fn fetch_powers(base_url: &str) -> EvccPowers {
    let url = format!("{}/api/state", base_url.trim_end_matches('/'));
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return EvccPowers::default();
    };
    let Ok(resp) = client.get(&url).send() else {
        return EvccPowers::default();
    };
    if !resp.status().is_success() {
        return EvccPowers::default();
    }
    let Ok(state) = resp.json::<EvccState>() else {
        return EvccPowers::default();
    };
    let charge = state
        .loadpoints
        .iter()
        .filter_map(|lp| lp.charge_power.or(lp.charge_power_camel))
        .sum::<f32>();
    EvccPowers {
        pv_w: state.pv.power,
        grid_w: state.grid.power,
        home_w: state.home_power,
        charge_w: if charge > 0.0 { Some(charge) } else { None },
    }
}
