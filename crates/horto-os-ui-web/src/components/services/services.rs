use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::{rewrite_service_url_host, UrlInfo};

include!(concat!(env!("OUT_DIR"), "/rangular/services_view.rs"));

#[component]
pub fn ServicesPanel(urls: Vec<UrlInfo>, api_base: String) -> impl IntoView {
    let mut names = Vec::with_capacity(urls.len());
    let mut hrefs = Vec::with_capacity(urls.len());
    let mut blurbs = Vec::with_capacity(urls.len());
    let mut ups = Vec::with_capacity(urls.len());

    for u in urls {
        names.push(u.name);
        hrefs.push(rewrite_service_url_host(&u.url, &api_base));
        blurbs.push(
            u.description
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "No catalog description yet.".into()),
        );
        ups.push(u.up);
    }

    let is_empty = names.is_empty();
    services_view(HostCell::new(ServicesHost {
        names,
        hrefs,
        blurbs,
        ups,
        is_empty,
    }))
}

struct ServicesHost {
    names: Vec<String>,
    hrefs: Vec<String>,
    blurbs: Vec<String>,
    ups: Vec<bool>,
    is_empty: bool,
}

impl Host for ServicesHost {
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "names" => Some(Value::List(
                self.names.iter().cloned().map(Value::Str).collect(),
            )),
            "isEmpty" => Some(Value::Bool(self.is_empty)),
            _ => None,
        }
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, HostError> {
        let Some(i) = arg_index(args) else {
            return Ok(Value::Unit);
        };
        Ok(match name {
            "hrefAt" => Value::Str(self.hrefs.get(i).cloned().unwrap_or_default()),
            "blurbAt" => Value::Str(self.blurbs.get(i).cloned().unwrap_or_default()),
            "isUp" => Value::Bool(self.ups.get(i).copied().unwrap_or(false)),
            "isDown" => Value::Bool(!self.ups.get(i).copied().unwrap_or(false)),
            "statusTitle" => Value::Str(
                if self.ups.get(i).copied().unwrap_or(false) {
                    "Port open on the box"
                } else {
                    "Port closed on the box"
                }
                .into(),
            ),
            _ => Value::Unit,
        })
    }
}

fn arg_index(args: &[Value]) -> Option<usize> {
    match args.first()? {
        Value::Num(n) if n.is_finite() && *n >= 0.0 => Some(*n as usize),
        _ => None,
    }
}
