//! Short descriptions for known Horto box services and container images.

/// Human description for a status-API service link name (`Homepage`, `EVCC`, …).
#[must_use]
pub fn describe_service(name: &str) -> Option<&'static str> {
    match normalize_key(name).as_str() {
        "homepage" => Some("Box dashboard (gethomepage) for apps and widgets."),
        "dockge" => Some("Compose stack manager for Docker apps on the box."),
        "cockpit" => Some("Host admin console (packages, logs, storage, network)."),
        "open-webui" | "openwebui" => Some("Local chat UI for on-box LLM backends."),
        "evcc" => Some("Home energy manager (chargers, PV, battery)."),
        "whisper" => Some("Speech-to-text service used by voice pipelines."),
        "deepseek" => Some("Local LLM endpoint (often via Open-WebUI)."),
        "piper" => Some("Text-to-speech engine (Wyoming / voice stack)."),
        "openwakeword" | "openwakeword-wyoming" => {
            Some("Wake-word detection for hands-free voice.")
        }
        "cloudflared" | "cloudflare-tunnel" | "cloudflare-tunnel-hos1" => {
            Some("Outbound tunnel to expose selected box services.")
        }
        _ => None,
    }
}

/// Human description from Docker container name and/or image reference.
#[must_use]
pub fn describe_container(names: &str, image: &str) -> Option<&'static str> {
    for part in names.split(',') {
        let key = normalize_key(part.trim().trim_start_matches('/'));
        if let Some(desc) = describe_service(&key) {
            return Some(desc);
        }
        if let Some(desc) = describe_by_alias(&key) {
            return Some(desc);
        }
    }
    describe_image(image)
}

fn describe_by_alias(key: &str) -> Option<&'static str> {
    match key {
        "wyoming-piper" | "piper" => describe_service("piper"),
        "whisper-cv" => describe_service("whisper"),
        "deepseek-npu" => describe_service("deepseek"),
        "openwakeword-wyoming" => describe_service("openwakeword"),
        _ => None,
    }
}

fn describe_image(image: &str) -> Option<&'static str> {
    let img = image.to_ascii_lowercase();
    if img.contains("gethomepage/homepage") {
        return describe_service("homepage");
    }
    if img.contains("louislam/dockge") {
        return describe_service("dockge");
    }
    if img.contains("open-webui") {
        return describe_service("open-webui");
    }
    if img.contains("evcc/evcc") {
        return describe_service("evcc");
    }
    if img.contains("wyoming-piper") || img.contains("rhasspy/wyoming-piper") {
        return describe_service("piper");
    }
    if img.contains("wyoming-openwakeword") {
        return describe_service("openwakeword");
    }
    if img.contains("whisper") {
        return describe_service("whisper");
    }
    if img.contains("deepseek") {
        return describe_service("deepseek");
    }
    if img.contains("cloudflared") {
        return describe_service("cloudflared");
    }
    if img.contains("nginx") {
        return Some("HTTP front (demo or reverse proxy).");
    }
    None
}

fn normalize_key(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::{describe_container, describe_service};

    #[test]
    fn known_services() {
        assert!(describe_service("Homepage").unwrap().contains("dashboard"));
        assert!(describe_service("Open-WebUI").is_some());
        assert!(describe_service("unknown-x").is_none());
    }

    #[test]
    fn known_containers_by_name_and_image() {
        assert!(describe_container("homepage", "nginx:alpine")
            .unwrap()
            .contains("dashboard"));
        assert!(
            describe_container("x", "ghcr.io/gethomepage/homepage:latest")
                .unwrap()
                .contains("dashboard")
        );
        assert!(describe_container("weird", "busybox:latest").is_none());
    }
}
