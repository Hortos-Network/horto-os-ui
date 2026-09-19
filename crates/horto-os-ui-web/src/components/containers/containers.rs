use leptos::prelude::*;

use crate::status::{dockge_href_for_container, ContainerInfo, UrlInfo};

#[component]
pub fn ContainersPanel(
    containers: Vec<ContainerInfo>,
    urls: Vec<UrlInfo>,
    api_base: String,
) -> impl IntoView {
    let empty = containers.is_empty();

    view! {
        <section class="containers panel" aria-label="Containers">
            <h2>"Containers"</h2>
            <ul class="containers__grid">
                {containers
                    .into_iter()
                    .map(|c| {
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
                        let tone = if up { "up" } else { "down" };
                        let href = dockge_href_for_container(
                            c.stack.as_deref(),
                            &name,
                            &urls,
                            &api_base,
                        );
                        let name_title = name.clone();
                        let image_title = image.clone();

                        if let Some(href) = href {
                            let class =
                                format!("containers__tile containers__tile--{tone} containers__tile--link");
                            view! {
                                <li>
                                    <a
                                        class=class
                                        href=href
                                        target="_blank"
                                        rel="noopener noreferrer"
                                        title="Open stack in Dockge"
                                    >
                                        <span class="containers__name" title=name_title>{name}</span>
                                        <span class="containers__blurb">{blurb}</span>
                                        <span class="containers__image" title=image_title>{image}</span>
                                        <span class="containers__status">{status}</span>
                                    </a>
                                </li>
                            }
                            .into_any()
                        } else {
                            let class = format!("containers__tile containers__tile--{tone}");
                            view! {
                                <li class=class>
                                    <span class="containers__name" title=name_title>{name}</span>
                                    <span class="containers__blurb">{blurb}</span>
                                    <span class="containers__image" title=image_title>{image}</span>
                                    <span class="containers__status">{status}</span>
                                </li>
                            }
                            .into_any()
                        }
                    })
                    .collect_view()}
            </ul>
            <Show when=move || empty fallback=|| ()>
                <p class="containers__muted">
                    "No containers reported (API may be dry-run off-box)."
                </p>
            </Show>
        </section>
    }
}

fn status_looks_up(status: &str) -> bool {
    let s = status.to_ascii_lowercase();
    s.contains("up") && !s.contains("exited") && !s.contains("dead") && !s.contains("created")
}
