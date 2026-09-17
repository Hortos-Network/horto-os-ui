use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::BoxStatus;

include!(concat!(env!("OUT_DIR"), "/rangular/box_status_view.rs"));

#[component]
pub fn BoxStatusPanel(status: BoxStatus) -> impl IntoView {
    let hostname = status.hostname.clone();
    let container_count = status.containers.len().to_string();
    let backup_label = if status.backup.initial_setup_present {
        "present"
    } else {
        "missing"
    }
    .to_string();
    let backup_count = status.backup.timestamped.len().to_string();

    box_status_view(HostCell::new(BoxStatusHost {
        hostname,
        container_count,
        backup_label,
        backup_count,
    }))
}

struct BoxStatusHost {
    hostname: String,
    container_count: String,
    backup_label: String,
    backup_count: String,
}

impl Host for BoxStatusHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "hostname" => Some(Value::Str(self.hostname.clone())),
            "containerCount" => Some(Value::Str(self.container_count.clone())),
            "backupLabel" => Some(Value::Str(self.backup_label.clone())),
            "backupCount" => Some(Value::Str(self.backup_count.clone())),
            _ => None,
        }
    }

    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, HostError> {
        Ok(Value::Unit)
    }
}
