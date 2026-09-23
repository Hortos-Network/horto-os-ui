//! TUI tab order and colored surface panels.

use horto_os_ui_shared::{McpHostProbe, SurfaceProbeReport, LONG_VERSION};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Visible TUI screens (Reboot only when remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Overview,
    Ssh,
    Cli,
    Api,
    Mcp,
    Reboot,
    Logs,
}

impl Screen {
    /// Tab titles for the current mode (digits match order).
    #[must_use]
    pub fn titles(remote: bool) -> Vec<&'static str> {
        if remote {
            vec![
                "1 Setup",
                "2 Overview",
                "3 SSH",
                "4 CLI",
                "5 API",
                "6 MCP",
                "7 Reboot",
                "8 Logs",
            ]
        } else {
            vec![
                "1 Setup",
                "2 Overview",
                "3 SSH",
                "4 CLI",
                "5 API",
                "6 MCP",
                "7 Logs",
            ]
        }
    }

    /// Ordered tab list for local or remote mode.
    #[must_use]
    pub fn all(remote: bool) -> Vec<Self> {
        if remote {
            vec![
                Self::Setup,
                Self::Overview,
                Self::Ssh,
                Self::Cli,
                Self::Api,
                Self::Mcp,
                Self::Reboot,
                Self::Logs,
            ]
        } else {
            vec![
                Self::Setup,
                Self::Overview,
                Self::Ssh,
                Self::Cli,
                Self::Api,
                Self::Mcp,
                Self::Logs,
            ]
        }
    }

    /// Zero-based index of this screen in [`Self::all`].
    #[must_use]
    pub fn index(self, remote: bool) -> usize {
        Self::all(remote)
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0)
    }

    /// Map digit `1`..`n` to a screen for the current mode.
    #[must_use]
    pub fn from_digit(d: char, remote: bool) -> Option<Self> {
        let n = d.to_digit(10)? as usize;
        if n == 0 {
            return None;
        }
        Self::all(remote).into_iter().nth(n - 1)
    }

    /// Next tab wrapping within the current mode's order.
    #[must_use]
    pub fn next(self, remote: bool) -> Self {
        let all = Self::all(remote);
        let i = self.index(remote);
        all[(i + 1) % all.len()]
    }

    /// Previous tab wrapping within the current mode's order.
    #[must_use]
    pub fn prev(self, remote: bool) -> Self {
        let all = Self::all(remote);
        let i = self.index(remote);
        all[(i + all.len() - 1) % all.len()]
    }
}

fn label(s: &str) -> Span<'static> {
    Span::styled(format!("{s:<14}"), Style::default().fg(Color::White))
}

fn value(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::Cyan))
}

fn muted(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::Cyan))
}

fn key_hint(s: &str) -> Span<'static> {
    Span::styled(
        s.to_owned(),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        title.to_owned(),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn kv(key: &str, val: Span<'static>) -> Line<'static> {
    Line::from(vec![label(key), val])
}

fn action(keys: &str, desc: &str) -> Line<'static> {
    Line::from(vec![key_hint(keys), Span::raw(format!("  {desc}"))])
}

/// Color a probe/status string (ok / fail / wait).
fn status_badge(raw: &str) -> Span<'static> {
    let lower = raw.to_ascii_lowercase();
    let (dot, color) = if matches!(lower.as_str(), "ok" | "active" | "true" | "n/a" | "process")
        || lower.contains("horto-os-ui")
        || (raw.starts_with('/') && !lower.contains("missing"))
    {
        ('●', Color::Green)
    } else if lower.contains("fail")
        || lower.contains("unreachable")
        || lower.contains("missing")
        || lower == "inactive"
        || lower == "false"
        || lower.contains("error")
        || lower.contains("auth")
        || lower.contains("required")
    {
        ('●', Color::Red)
    } else if lower.contains("probing") || lower.contains("pending") {
        ('●', Color::Cyan)
    } else {
        ('●', Color::White)
    };
    Span::styled(
        format!("{dot} {raw}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn bool_badge(ok: bool, yes: &str, no: &str) -> Span<'static> {
    if ok {
        Span::styled(
            format!("● {yes}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!("● {no}"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    }
}

/// Format SSH panel from a probe (or empty state).
#[must_use]
pub fn panel_ssh(
    remote: bool,
    host: &str,
    report: Option<&SurfaceProbeReport>,
    pending_status: &str,
    install_ssh_key: bool,
) -> Vec<Line<'static>> {
    if !remote {
        return vec![
            kv("Mode", value("embedded (on box)")),
            kv("SSH", muted("n/a")),
        ];
    }
    let mut lines = vec![section("Connection"), kv("Host", value(host.to_owned()))];
    match report {
        None => {
            lines.push(kv("Status", status_badge(pending_status)));
            lines.push(kv("Key auth", muted("…")));
        }
        Some(r) => {
            lines.push(kv("Status", status_badge(&r.ssh.status)));
            lines.push(kv(
                "Key auth",
                bool_badge(r.ssh.key_ok, "BatchMode ok", "password may be needed"),
            ));
        }
    }
    lines.push(blank());
    lines.push(section("Actions"));
    lines.push(action("Enter", "Edit OpenSSH Host"));
    lines.push(action("f", "Fetch SSH status"));
    if install_ssh_key {
        lines.push(action("i", "Install this PC key on the box"));
    } else {
        lines.push(Line::from(muted(
            "Key install off · start with --install-ssh-key to enable",
        )));
    }
    lines
}

/// Format CLI panel.
#[must_use]
pub fn panel_cli(
    remote: bool,
    report: Option<&SurfaceProbeReport>,
    pending_box: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        section("Versions"),
        kv("Local", value(LONG_VERSION.to_owned())),
    ];
    if !remote {
        lines.push(kv("Box", value("local (embedded)")));
        return lines;
    }
    match report {
        None => {
            lines.push(kv("Box", status_badge(pending_box)));
            lines.push(blank());
            lines.push(section("Actions"));
            lines.push(action("f", "Fetch CLI status"));
        }
        Some(r) => {
            lines.push(kv("Box", value(r.cli.status.as_label().to_owned())));
            lines.push(kv(
                "Match",
                bool_badge(r.cli.current, "up to date", "out of date"),
            ));
            lines.push(blank());
            lines.push(section("Actions"));
            lines.push(action("Enter", "Sync CLI to box (s0)"));
            lines.push(action("f", "Fetch CLI status"));
        }
    }
    lines
}

/// Format API panel.
#[must_use]
pub fn panel_api(report: Option<&SurfaceProbeReport>) -> Vec<Line<'static>> {
    report.map_or_else(
        || {
            vec![
                section("Status API"),
                Line::from(muted("No data yet")),
                blank(),
                section("Actions"),
                action("f", "Fetch API status"),
            ]
        },
        |r| {
            let unit = if r.api.unit.is_empty() {
                "-"
            } else {
                r.api.unit.as_str()
            };
            vec![
                section("Status API"),
                kv("URL", value(r.api.url.clone())),
                kv("Health", status_badge(&r.api.health)),
                kv("/v1/status", status_badge(&r.api.status)),
                kv(
                    "Token file",
                    bool_badge(r.api.local_token, "present", "missing"),
                ),
                kv("Unit", status_badge(unit)),
                blank(),
                section("Actions"),
                action("f", "Fetch API status"),
            ]
        },
    )
}

/// Format MCP panel (PC + box; both stdio runtime and HTTP).
#[must_use]
pub fn panel_mcp(report: Option<&SurfaceProbeReport>) -> Vec<Line<'static>> {
    report.map_or_else(
        || {
            vec![
                Line::from(muted("No data yet")),
                blank(),
                section("Actions"),
                action("f", "Fetch MCP status"),
            ]
        },
        |r| {
            let mut lines = Vec::new();
            lines.extend(mcp_host_section("PC", &r.mcp_pc));
            lines.push(blank());
            lines.extend(mcp_host_section("Box", &r.mcp_box));
            lines.push(blank());
            lines.push(section("Actions"));
            lines.push(action("f", "Fetch MCP status"));
            lines
        },
    )
}

fn mcp_host_section(title: &str, p: &McpHostProbe) -> Vec<Line<'static>> {
    let docker = p.docker.as_deref().unwrap_or("missing");
    let binary = p.binary.as_deref().unwrap_or("missing");
    let unit = if p.unit.is_empty() {
        "-"
    } else {
        p.unit.as_str()
    };
    vec![
        section(title),
        kv("Image", status_badge(docker)),
        kv("Binary", status_badge(binary)),
        kv("Unit", status_badge(unit)),
        kv("HTTP (optional)", status_badge(&p.http_reach)),
        Line::from(value(p.http_url.clone())),
    ]
}

/// Format Reboot panel.
#[must_use]
pub fn panel_reboot(host: &str) -> Vec<Line<'static>> {
    vec![
        kv("Target", value(host.to_owned())),
        kv("Method", value("SSH + sudo on the box")),
        blank(),
        section("Actions"),
        action("Enter", "Confirm reboot"),
    ]
}

/// Overview facts from probe + optional doctor lines.
#[must_use]
pub fn panel_overview_remote(
    host: &str,
    report: Option<&SurfaceProbeReport>,
    extra: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![section("Remote"), kv("Host", value(host.to_owned()))];
    match report {
        None => {
            lines.push(kv("Surfaces", status_badge("probing…")));
        }
        Some(r) => {
            lines.push(kv("Local CLI", value(r.local_version.clone())));
            lines.push(kv("Box CLI", value(r.cli.status.as_label().to_owned())));
            lines.push(blank());
            lines.push(section("Surfaces"));
            lines.push(kv("SSH", status_badge(&r.ssh.status)));
            lines.push(kv("API", status_badge(&r.api.health)));
            lines.push(kv("MCP PC HTTP", status_badge(&r.mcp_pc.http_reach)));
            lines.push(kv("MCP box HTTP", status_badge(&r.mcp_box.http_reach)));
        }
    }
    if !extra.is_empty() {
        lines.push(blank());
        lines.push(section("Doctor"));
        for line in extra.lines() {
            if line.is_empty() {
                lines.push(blank());
            } else {
                lines.push(Line::from(muted(line.to_owned())));
            }
        }
    }
    lines.push(blank());
    lines.push(section("Actions"));
    lines.push(action("f", "Fetch overview surfaces"));
    lines
}

/// Plain overview body (local / embedded) as styled lines.
#[must_use]
pub fn panel_lines_from_plain(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| Line::from(Span::raw(l.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_join(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn remote_tab_order_ends_with_logs() {
        let t = Screen::titles(true);
        assert_eq!(t.last().copied(), Some("8 Logs"));
        assert!(t.iter().any(|x| x.contains("Reboot")));
        assert_eq!(Screen::from_digit('8', true), Some(Screen::Logs));
        assert_eq!(Screen::from_digit('7', false), Some(Screen::Logs));
        assert_eq!(Screen::Setup.next(true), Screen::Overview);
        assert_eq!(Screen::Logs.prev(true), Screen::Reboot);
    }

    #[test]
    fn panels_pending_use_probing_not_question() {
        let cli = lines_join(&panel_cli(true, None, "probing..."));
        assert!(cli.contains("probing..."));
        assert!(!cli.contains('?'));
        assert!(!cli.contains("missing"));
        let ssh = lines_join(&panel_ssh(true, "horto", None, "probing...", false));
        assert!(ssh.contains("probing..."));
        assert!(ssh.contains("Key install off"));
        assert!(!ssh.contains("when started with"));
    }

    #[test]
    fn ssh_key_install_hint_when_enabled() {
        let ssh = lines_join(&panel_ssh(true, "horto", None, "ok", true));
        assert!(ssh.contains("Install this PC key"));
    }
}
