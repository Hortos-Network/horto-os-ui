use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::UrlInfo;

include!(concat!(env!("OUT_DIR"), "/rangular/services_view.rs"));

#[component]
pub fn ServicesPanel(urls: Vec<UrlInfo>) -> impl IntoView {
    let empty = urls.is_empty();
    let lines: Vec<String> = urls
        .into_iter()
        .map(|u| format!("{}\n{}", u.name, u.url))
        .collect();
    services_view(HostCell::new(ServicesHost { lines, empty }))
}

struct ServicesHost {
    lines: Vec<String>,
    empty: bool,
}

impl Host for ServicesHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "lines" => Some(Value::List(
                self.lines.iter().cloned().map(Value::Str).collect(),
            )),
            "empty" => Some(Value::Bool(self.empty)),
            _ => None,
        }
    }

    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, HostError> {
        Ok(Value::Unit)
    }
}
