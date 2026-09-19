use leptos::prelude::*;

use crate::status::{rewrite_service_url_host, UrlInfo};

#[component]
pub fn ServicesPanel(urls: Vec<UrlInfo>, api_base: String) -> impl IntoView {
    let empty = urls.is_empty();
    let rows: Vec<(String, String, bool, String)> = urls
        .into_iter()
        .map(|u| {
            let href = rewrite_service_url_host(&u.url, &api_base);
            let blurb = u
                .description
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "No catalog description yet.".into());
            (u.name, href, u.up, blurb)
        })
        .collect();

    view! {
        <section class="services panel" aria-label="Services">
            <h2>"Services"</h2>
            <ul class="services__list">
                {rows
                    .into_iter()
                    .map(|(name, href, up, blurb)| {
                        let status = if up { "up" } else { "down" };
                        let title = if up {
                            "Port open on the box"
                        } else {
                            "Port closed on the box"
                        };
                        let href_attr = href.clone();
                        view! {
                            <li class="services__row">
                                <span class="services__name">{name}</span>
                                <span class="services__blurb">{blurb}</span>
                                <a
                                    class="services__url"
                                    href=href_attr
                                    target="_blank"
                                    rel="noopener noreferrer"
                                >
                                    {href}
                                </a>
                                <span
                                    class=format!("services__dot services__dot--{status}")
                                    title=title
                                    aria-label=title
                                    role="img"
                                ></span>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
            <Show when=move || empty fallback=|| ()>
                <p class="services__muted">"No service URLs in the status payload."</p>
            </Show>
        </section>
    }
}
