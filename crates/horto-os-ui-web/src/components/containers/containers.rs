use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::ContainerInfo;

include!(concat!(env!("OUT_DIR"), "/rangular/containers_view.rs"));

#[component]
pub fn ContainersPanel(containers: Vec<ContainerInfo>) -> impl IntoView {
    let empty = containers.is_empty();
    let rows: Vec<String> = containers
        .into_iter()
        .map(|c| {
            let image = if c.image.is_empty() {
                "(image unknown)".into()
            } else {
                c.image
            };
            format!("{}\n{image}\n{}", c.names, c.status)
        })
        .collect();
    containers_view(HostCell::new(ContainersHost { rows, empty }))
}

struct ContainersHost {
    rows: Vec<String>,
    empty: bool,
}

impl Host for ContainersHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "rows" => Some(Value::List(
                self.rows.iter().cloned().map(Value::Str).collect(),
            )),
            "empty" => Some(Value::Bool(self.empty)),
            _ => None,
        }
    }

    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, HostError> {
        Ok(Value::Unit)
    }
}
