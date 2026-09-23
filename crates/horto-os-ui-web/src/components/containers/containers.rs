use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};

use crate::status::{dockge_href_for_container, ContainerInfo, UrlInfo};

include!(concat!(env!("OUT_DIR"), "/rangular/containers_view.rs"));

#[component]
pub fn ContainersPanel(
    containers: Vec<ContainerInfo>,
    urls: Vec<UrlInfo>,
    api_base: String,
) -> impl IntoView {
    let mut names = Vec::with_capacity(containers.len());
    let mut images = Vec::with_capacity(containers.len());
    let mut statuses = Vec::with_capacity(containers.len());
    let mut blurbs = Vec::with_capacity(containers.len());
    let mut ups = Vec::with_capacity(containers.len());
    let mut hrefs = Vec::with_capacity(containers.len());

    for c in containers {
        let name = if c.names.is_empty() {
            "(unnamed)".into()
        } else {
            c.names
        };
        let image = if c.image.is_empty() {
            "(image unknown)".into()
        } else {
            c.image
        };
        let status = if c.status.is_empty() {
            "unknown".into()
        } else {
            c.status
        };
        let blurb = c
            .description
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "No catalog description yet.".into());
        let up = status_looks_up(&status);
        let href = dockge_href_for_container(c.stack.as_deref(), &name, &urls, &api_base)
            .unwrap_or_default();
        names.push(name);
        images.push(image);
        statuses.push(status);
        blurbs.push(blurb);
        ups.push(up);
        hrefs.push(href);
    }

    let is_empty = names.is_empty();
    containers_view(HostCell::new(ContainersHost {
        names,
        images,
        statuses,
        blurbs,
        ups,
        hrefs,
        is_empty,
    }))
}

struct ContainersHost {
    names: Vec<String>,
    images: Vec<String>,
    statuses: Vec<String>,
    blurbs: Vec<String>,
    ups: Vec<bool>,
    hrefs: Vec<String>,
    is_empty: bool,
}

impl Host for ContainersHost {
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
            "imageAt" => Value::Str(self.images.get(i).cloned().unwrap_or_default()),
            "statusAt" => Value::Str(self.statuses.get(i).cloned().unwrap_or_default()),
            "blurbAt" => Value::Str(self.blurbs.get(i).cloned().unwrap_or_default()),
            "hrefAt" => Value::Str(self.hrefs.get(i).cloned().unwrap_or_default()),
            "isUp" => Value::Bool(self.ups.get(i).copied().unwrap_or(false)),
            "isDown" => Value::Bool(!self.ups.get(i).copied().unwrap_or(false)),
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

fn status_looks_up(status: &str) -> bool {
    let s = status.to_ascii_lowercase();
    s.contains("up") && !s.contains("exited") && !s.contains("dead") && !s.contains("created")
}
