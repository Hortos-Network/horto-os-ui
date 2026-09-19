//! Synthetic samples so the KPI board can demo GPUI charts without live EVCC.

use crate::history::MetricSample;

/// Time-varying fake metrics (smooth curves + a bit of noise).
#[must_use]
pub fn sample_at(tick: u32) -> MetricSample {
    let t = f64::from(tick);
    let wave = |amp: f64, period: f64, phase: f64, base: f64| -> f32 {
        let v = base + amp * (t / period + phase).sin();
        v as f32
    };
    let noise = |seed: u32| -> f32 {
        let x = tick.wrapping_mul(1103515245).wrapping_add(seed);
        (x % 21) as f32 - 10.0
    };

    // Solar-ish day curve on top of a slow sine.
    let day = ((t / 40.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
    let pv = (400.0 + 2800.0 * day + f64::from(noise(1)) * 8.0).max(0.0) as f32;
    let home = wave(180.0, 11.0, 0.4, 900.0) + noise(2) * 3.0;
    let charge = {
        let pulse = ((t / 18.0).sin() > 0.35) as i32 as f32;
        (pulse * (2200.0 + f64::from(noise(3)) * 40.0) as f32).max(0.0)
    };
    let grid = home + charge - pv;

    let containers = (9.0 + (t / 25.0).sin() * 1.2 + f64::from(noise(4)) * 0.05)
        .round()
        .clamp(7.0, 11.0) as f32;
    let services = (8.0 + (t / 30.0).cos() * 0.8).round().clamp(6.0, 9.0) as f32;
    let setup = wave(4.0, 50.0, 1.1, 92.0).clamp(80.0, 100.0);
    let doctor = wave(3.0, 45.0, 2.0, 88.0).clamp(75.0, 100.0);
    let leases = (24.0 + (t / 14.0).sin() * 6.0 + f64::from(noise(5)) * 0.2)
        .round()
        .clamp(12.0, 40.0) as f32;

    MetricSample {
        containers_up: containers,
        services_up: services,
        setup_done_pct: setup,
        doctor_pct: doctor,
        leases,
        pv_w: Some(pv),
        grid_w: Some(grid),
        home_w: Some(home),
        charge_w: Some(charge),
    }
}

/// Prefill a history buffer with `len` synthetic ticks ending at `end_tick`.
pub fn seed_history(history: &mut crate::history::LiveHistory, end_tick: u32, len: usize) {
    let len = len.max(2) as u32;
    let start = end_tick.saturating_sub(len - 1);
    for tick in start..=end_tick {
        history.push_sample(&sample_at(tick));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_finite() {
        let s = sample_at(42);
        assert!(s.pv_w.unwrap().is_finite());
        assert!(s.grid_w.unwrap().is_finite());
        assert!(s.home_w.unwrap().is_finite());
        assert!((0.0..=100.0).contains(&s.setup_done_pct));
    }
}
