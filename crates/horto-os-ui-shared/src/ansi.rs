//! Strip terminal escape sequences and noisy capture from remote setup output.

/// Remove CSI / OSC-style ANSI escapes so Install / Logs stay readable.
#[must_use]
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for x in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&x) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                for x in chars.by_ref() {
                    if x == '\u{7}' || x == '\u{1b}' {
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Clean SSH-captured setup stdout/stderr for Install / Logs.
///
/// Drops apt progress noise and interactive prompt labels; unwraps tracing
/// `INFO target: message` lines to the message body only.
#[must_use]
pub fn sanitize_captured_setup_log(raw: &str) -> String {
    let raw = strip_ansi(raw);
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let Some(kept) = keep_setup_log_line(line) else {
            continue;
        };
        if out.last().is_some_and(|prev| prev == &kept) {
            continue;
        }
        out.push(kept);
    }
    out.join("\n")
}

fn keep_setup_log_line(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // Prefer a tracing payload if prompt labels were glued onto the same line.
    let t = tracing_payload_start(t).unwrap_or(t);
    if is_setup_log_noise(t) {
        return None;
    }
    Some(unwrap_tracing_message(t).to_owned())
}

fn tracing_payload_start(line: &str) -> Option<&str> {
    for marker in [" ERROR ", " WARN ", " INFO "] {
        if let Some(i) = line.find(marker) {
            // Require an ISO-ish timestamp before the level (remote fmt lines).
            let before = &line[..i];
            if before.contains('T') && before.contains('Z') {
                return Some(line[i + 1..].trim_start()); // keep "ERROR …" / "INFO …"
            }
        }
    }
    None
}

fn unwrap_tracing_message(line: &str) -> &str {
    for marker in ["ERROR ", "WARN ", "INFO "] {
        let Some(rest) = line.strip_prefix(marker).or_else(|| {
            line.find(marker)
                .map(|i| line[i + marker.len()..].trim_start())
        }) else {
            continue;
        };
        // "target: message" or "target:message"
        if let Some(colon) = rest.find(": ") {
            return rest[colon + 2..].trim();
        }
        return rest.trim();
    }
    line
}

fn is_setup_log_noise(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("Hit:")
        || t.starts_with("Get:")
        || t.starts_with("Ign:")
        || t.starts_with("Reading package lists")
        || t.starts_with("Building dependency")
        || t.starts_with("Reading state information")
        || t.starts_with("WARNING: apt does not")
        || t.starts_with("Summary:")
        || t.starts_with("Upgrading:")
        || t.starts_with("Installing:")
        || t.starts_with("Removing:")
        || t.starts_with("Not Upgrading:")
        || t.contains("already the newest version")
        || t.starts_with("* Applying /")
        || t.starts_with("kernel.")
        || t.starts_with("net.ipv4.")
        || t.starts_with("net.ipv6.")
        || t.starts_with("vm.")
        || t.starts_with("Synchronizing state of")
        || t.starts_with("Executing: /usr/lib/systemd")
    {
        return true;
    }
    // Interactive prompt labels (defaults applied; not answered on Desktop).
    if (t.contains("[y/N]") || t.contains("[Y/n]") || t.contains("]: "))
        && (t.contains("hostname")
            || t.contains("OS type")
            || t.contains("NPU")
            || t.contains("RAM size")
            || t.contains("Install type")
            || t.contains("IOT-LAN")
            || t.contains("Public URL")
            || t.contains("Cloudflare")
            || t.contains("WiFi interface")
            || t.contains("Apply NAT")
            || t.contains("masquerade")
            || t.contains("reboot"))
        && !t.contains("==>")
        && !t.contains("[plan]")
        && tracing_payload_start(t).is_none()
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{sanitize_captured_setup_log, strip_ansi};

    #[test]
    fn strip_ansi_removes_csi_colors() {
        let raw = "\u{1b}[2m2026-09-23T19:50:39Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m apt update";
        assert_eq!(strip_ansi(raw), "2026-09-23T19:50:39Z  INFO apt update");
    }

    #[test]
    fn strip_ansi_leaves_plain_text() {
        assert_eq!(strip_ansi("plain\nline"), "plain\nline");
    }

    #[test]
    fn sanitize_drops_apt_and_prompts_keeps_steps() {
        let raw = "\
Hit:1 http://deb.debian.org/debian trixie InRelease\n\
WARNING: apt does not have a stable CLI interface. Use with caution in scripts.\n\
Device hostname [horto]: OS type (debian/armbian) [armbian]: \n\
Apply NAT / masquerade iptables rules now? [y/N]: \n\
2026-09-23T20:04:39.878235Z  INFO horto_os_ui_shared::context: ==> Install base packages (s1)\n\
2026-09-23T20:04:39.878320Z  INFO horto_os_ui_shared::context: apt update\n\
WiFi interface (none = Ethernet-only) [none]: 2026-09-23T20:04:44.665796Z  INFO horto_os_ui_shared::context: WIFI_INTERFACE=none; Ethernet-only IoT-LAN (hostapd skipped)\n";
        let clean = sanitize_captured_setup_log(raw);
        assert!(clean.contains("==> Install base packages (s1)"));
        assert!(clean.contains("apt update"));
        assert!(clean.contains("WIFI_INTERFACE=none"));
        assert!(!clean.contains("Hit:"));
        assert!(!clean.contains("WARNING: apt"));
        assert!(!clean.contains("Device hostname"));
        assert!(!clean.contains("Apply NAT"));
    }
}
