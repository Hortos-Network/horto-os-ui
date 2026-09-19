//! Ring buffers for live chart history.

#[derive(Debug, Clone)]
pub struct Series {
    pub points: Vec<f32>,
    capacity: usize,
}

impl Series {
    #[must_use]
    pub fn new(_label: &'static str, capacity: usize) -> Self {
        Self {
            points: Vec::with_capacity(capacity),
            capacity: capacity.max(2),
        }
    }

    pub fn push(&mut self, value: f32) {
        if self.points.len() >= self.capacity {
            self.points.remove(0);
        }
        self.points.push(value);
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetricSample {
    pub containers_up: f32,
    pub containers_total: f32,
    pub services_up: f32,
    pub services_total: f32,
    pub setup_done_pct: f32,
    pub doctor_pct: f32,
    pub leases: f32,
    pub pv_w: Option<f32>,
    pub grid_w: Option<f32>,
    pub home_w: Option<f32>,
    pub charge_w: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct LiveHistory {
    pub containers_up: Series,
    pub services_up: Series,
    pub setup_pct: Series,
    pub doctor_pct: Series,
    pub leases: Series,
    pub pv_w: Series,
    pub grid_w: Series,
    pub home_w: Series,
    pub charge_w: Series,
}

impl LiveHistory {
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            containers_up: Series::new("Containers up", cap),
            services_up: Series::new("Services up", cap),
            setup_pct: Series::new("Setup %", cap),
            doctor_pct: Series::new("Doctor %", cap),
            leases: Series::new("DHCP leases", cap),
            pv_w: Series::new("PV W", cap),
            grid_w: Series::new("Grid W", cap),
            home_w: Series::new("Home W", cap),
            charge_w: Series::new("Charge W", cap),
        }
    }

    pub fn push_sample(&mut self, s: &MetricSample) {
        self.containers_up.push(s.containers_up);
        self.services_up.push(s.services_up);
        self.setup_pct.push(s.setup_done_pct);
        self.doctor_pct.push(s.doctor_pct);
        self.leases.push(s.leases);
        if let Some(v) = s.pv_w {
            self.pv_w.push(v);
        }
        if let Some(v) = s.grid_w {
            self.grid_w.push(v);
        }
        if let Some(v) = s.home_w {
            self.home_w.push(v);
        }
        if let Some(v) = s.charge_w {
            self.charge_w.push(v);
        }
    }
}
