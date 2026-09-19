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
    #[serde(alias = "pvPower")]
    pv_power: Option<f32>,
    #[serde(default)]
    #[serde(alias = "gridPower")]
    grid_power: Option<f32>,
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
///
/// Returns `Err` with a short reason when the endpoint is missing, HTML, or not JSON
/// (common when the linked "EVCC" port is only a reverse-proxy stub).
pub fn fetch_powers(base_url: &str) -> Result<EvccPowers, String> {
    let url = format!("{}/api/state", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("EVCC client: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("EVCC {url}: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| format!("EVCC {url}: read body: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "EVCC {url}: HTTP {status} (need a real EVCC /api/state, not a static proxy)"
        ));
    }
    let trimmed = body.trim_start();
    if !trimmed.starts_with('{') {
        return Err(format!(
            "EVCC {url}: not JSON (got HTML/text; port is not serving EVCC API)"
        ));
    }
    let state: EvccState =
        serde_json::from_str(&body).map_err(|e| format!("EVCC {url}: parse: {e}"))?;
    let charge: f32 = state
        .loadpoints
        .iter()
        .filter_map(|lp| lp.charge_power.or(lp.charge_power_camel))
        .sum();
    Ok(EvccPowers {
        pv_w: state.pv.power.or(state.pv_power),
        grid_w: state.grid.power.or(state.grid_power),
        home_w: state.home_power,
        charge_w: Some(charge),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_evcc_shape() {
        let raw = r#"{
            "pv": {"power": 1200.5},
            "grid": {"power": -200.0},
            "homePower": 900.0,
            "loadpoints": [{"chargePower": 110.0}, {"charge_power": 40.0}]
        }"#;
        let state: EvccState = serde_json::from_str(raw).unwrap();
        assert!((state.pv.power.unwrap() - 1200.5).abs() < f32::EPSILON);
        assert!((state.home_power.unwrap() - 900.0).abs() < f32::EPSILON);
        let charge: f32 = state
            .loadpoints
            .iter()
            .filter_map(|lp| lp.charge_power.or(lp.charge_power_camel))
            .sum();
        assert!((charge - 150.0).abs() < f32::EPSILON);
    }
}
