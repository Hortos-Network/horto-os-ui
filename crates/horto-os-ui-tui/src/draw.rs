//! Ratatui draw helpers: layout, footer, panels, help overlay.

use horto_os_ui_shared::{GIT_COMMIT, VERSION};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};

#[cfg(test)]
use crate::app::BoxCliView;
use crate::app::{App, Modal};
use crate::prompt::{draw_confirm, draw_rebooting, draw_secret_input, draw_text_input};
use crate::tabs::Screen;

/// Footer label for local tip version and optional box CLI status (tests).
#[cfg(test)]
pub fn footer_cli_label(cli_local: &str, remote: bool, box_cli: &BoxCliView) -> String {
    if remote {
        format!("local={cli_local} box={}", box_cli.as_label())
    } else {
        format!("local={cli_local}")
    }
}

/// Session state + last user message (user message in white).
fn footer_status_line(app: &App) -> Line<'static> {
    let mode = if app.apply { "APPLY" } else { "PLAN" };
    let mode_style = if app.apply {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };
    let value_style = Style::default().fg(Color::Cyan);
    let mut spans = vec![
        Span::styled(format!("[{mode}]"), mode_style),
        Span::raw(" pipeline="),
        Span::styled(app.kind.as_str().to_owned(), value_style),
        Span::raw(" · local="),
        Span::styled(app.cli_local.clone(), value_style),
    ];
    if app.is_remote() {
        spans.push(Span::raw(" box="));
        spans.push(Span::styled(app.box_cli.as_label().to_owned(), value_style));
    }
    if !app.message.is_empty() {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            app.message.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

fn footer_key(label: &str) -> Span<'static> {
    Span::styled(
        label.to_owned(),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_muted(text: &str) -> Span<'static> {
    Span::styled(text.to_owned(), Style::default().fg(Color::DarkGray))
}

/// Panel / block title (same orange as the selected tab).
fn panel_title(name: &str) -> Span<'static> {
    Span::styled(
        name.to_owned(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_hints_line(app: &App) -> Line<'static> {
    if app.help_open {
        return Line::from(vec![
            footer_muted("Esc or "),
            footer_key("?"),
            footer_muted(" close help"),
        ]);
    }
    if let Some(line) = footer_for_modal(app.modal.as_ref()) {
        return line;
    }
    footer_for_screen(app.screen)
}

fn footer_for_modal(modal: Option<&Modal>) -> Option<Line<'static>> {
    match modal {
        Some(Modal::Confirm(_)) => Some(Line::from(vec![
            footer_key("Enter"),
            footer_muted("/"),
            footer_key("y"),
            footer_muted(" confirm · "),
            footer_key("Esc"),
            footer_muted("/"),
            footer_key("n"),
            footer_muted(" cancel · "),
            footer_key("Ctrl+C"),
            footer_muted(" quit"),
        ])),
        Some(Modal::TextHost(_)) => Some(Line::from(vec![
            footer_muted("Type host · "),
            footer_key("Tab"),
            footer_muted(" cycle · "),
            footer_key("Enter"),
            footer_muted(" submit · "),
            footer_key("Esc"),
            footer_muted(" cancel · "),
            footer_key("Ctrl+C"),
            footer_muted(" quit"),
        ])),
        Some(Modal::SudoPassword(_)) => Some(Line::from(vec![
            footer_muted("Sudo password · "),
            footer_key("Enter"),
            footer_muted(" submit · "),
            footer_key("Esc"),
            footer_muted(" cancel · "),
            footer_key("Ctrl+C"),
            footer_muted(" quit"),
        ])),
        Some(Modal::Rebooting) => Some(Line::from(vec![
            footer_muted("Rebooting… · "),
            footer_key("Esc"),
            footer_muted(" close · "),
            footer_key("Ctrl+C"),
            footer_muted(" quit"),
        ])),
        None => None,
    }
}

fn footer_for_screen(screen: Screen) -> Line<'static> {
    match screen {
        Screen::Setup => footer_key_line(&[
            ("j/k", "/"),
            ("↑/↓", " select · "),
            ("←/→", " tabs · "),
            ("Enter", " run · "),
            ("a", " all · "),
            ("b", " backup · "),
            ("p", " pipeline · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
        Screen::Logs => footer_key_line(&[
            ("c", " clear · "),
            ("←/→", " tabs · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh · "),
            ("B", " disk · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
        Screen::Ssh => footer_key_line(&[
            ("Enter", " edit host · "),
            ("f", " fetch · "),
            ("i", " key · "),
            ("←/→", " tabs · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh all · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
        Screen::Cli => footer_key_line(&[
            ("Enter", " sync CLI · "),
            ("f", " fetch · "),
            ("←/→", " tabs · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh all · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
        Screen::Overview | Screen::Api | Screen::Mcp => footer_key_line(&[
            ("f", " fetch · "),
            ("←/→", " tabs · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh all · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
        Screen::Reboot => footer_key_line(&[
            ("Enter", " reboot · "),
            ("←/→", " tabs · "),
            ("Tab", " plan/apply · "),
            ("r", " refresh · "),
            ("?", " help · "),
            ("q", " quit"),
        ]),
    }
}

fn footer_key_line(parts: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(parts.len() * 2);
    for &(key, tail) in parts {
        spans.push(footer_key(key));
        spans.push(footer_muted(tail));
    }
    Line::from(spans)
}

fn tab_title_line(title: &str) -> Line<'static> {
    let mut chars = title.chars();
    let Some(digit) = chars.next() else {
        return Line::from(title.to_owned());
    };
    let rest: String = chars.collect();
    Line::from(vec![
        Span::styled(
            digit.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(rest),
    ])
}

/// Draw the full TUI frame: tabs, active panel, footer, and modal overlays.
pub fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(f.area());

    let remote = app.is_remote();
    let titles = Screen::titles(remote)
        .into_iter()
        .map(tab_title_line)
        .collect::<Vec<_>>();
    let idx = app.screen.index(remote);
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("horto-tui/{VERSION}/{GIT_COMMIT}")),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    match app.screen {
        Screen::Setup => draw_setup(f, app, chunks[1]),
        Screen::Logs => draw_logs(f, app, chunks[1]),
        Screen::Overview
        | Screen::Ssh
        | Screen::Cli
        | Screen::Api
        | Screen::Mcp
        | Screen::Reboot => draw_panel(f, app, chunks[1]),
    }

    draw_footer(f, app, chunks[2]);

    if app.help_open {
        draw_help(f);
    }
    match &app.modal {
        Some(Modal::Confirm(kind)) => {
            draw_confirm(f, kind.title(), &kind.body());
        }
        Some(Modal::TextHost(input)) => {
            draw_text_input(f, input);
        }
        Some(Modal::SudoPassword(input)) => {
            draw_secret_input(f, input);
        }
        Some(Modal::Rebooting) => {
            draw_rebooting(f);
        }
        None => {}
    }
}

/// Footer: status (session + white user message) · keys. Cleared each frame.
fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Status");
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let status = pad_footer_line(footer_status_line(app), inner.width);
    f.render_widget(
        Paragraph::new(status),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );
    if inner.height >= 2 {
        let hints = pad_footer_line(footer_hints_line(app), inner.width);
        f.render_widget(
            Paragraph::new(hints),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn pad_footer_line(line: Line<'static>, width: u16) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::default();
    }
    let used = line.width();
    if used > width {
        let s = line.to_string();
        let mut out = String::new();
        for ch in s.chars() {
            if out.chars().count() + 1 >= width {
                break;
            }
            out.push(ch);
        }
        if s.chars().count() > width {
            if out.chars().count() == width {
                out.pop();
            }
            out.push('…');
        }
        return Line::from(out);
    }
    if used < width {
        let mut line = line;
        line.spans.push(Span::raw(" ".repeat(width - used)));
        return line;
    }
    line
}

fn draw_help(f: &mut Frame) {
    let area = centered_rect(70, 80, f.area());
    f.render_widget(Clear, area);
    let body = [
        "Horto TUI help",
        "",
        "q / Esc / Ctrl+C   Quit",
        "?                  Toggle this help",
        "Left / Right       Previous / next tab",
        "1-8                Jump to tab (remote: 7 Reboot, 8 Logs)",
        "j k / Up / Down    Move step selection (Setup)",
        "p                  Toggle full / minimal pipeline",
        "Tab                Toggle plan / apply",
        "Enter              Setup: run step · SSH: edit Host · other surfaces: action",
        "e / i              SSH: edit Host / install key (--install-ssh-key)",
        "a                  Run all pipeline steps",
        "b / B              Timestamped /etc backup / disk probe",
        "r                  Refresh surfaces (SSH/CLI/API/MCP; background)",
        "c                  Clear Logs (on Logs tab)",
        "y / n              Confirm / cancel (modals)",
        "",
        "Remote open paints first, then refreshes SSH/CLI/API/MCP in the background.",
        "Press r to refresh without blocking. Confirm and Host edit use on-screen dialogs.",
        "Mouse capture is off so you can select and copy text.",
        "Press Esc or ? to close.",
    ]
    .join("\n");
    let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

fn draw_setup(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .status_lines
        .iter()
        .map(|l| ListItem::new(setup_step_line(l)))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(ratatui::widgets::Padding::new(0, 0, 1, 0))
                .title(Line::from(vec![
                    panel_title("Steps"),
                    Span::raw(" ("),
                    footer_key("Enter"),
                    Span::raw(" run · "),
                    footer_key("a"),
                    Span::raw(" = all · "),
                    footer_key("*"),
                    Span::raw(" = destructive)"),
                ])),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, area, &mut app.step_state);
}

/// Color step id and status separately: `s0 | pending | title`.
pub fn setup_step_line(raw: &str) -> Line<'static> {
    let parts: Vec<&str> = raw.splitn(3, " | ").collect();
    if parts.len() < 3 {
        return Line::from(Span::raw(raw.to_owned()));
    }
    let id = parts[0];
    let status = parts[1];
    let title = parts[2];
    let status_style = match status {
        "done" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        "pending" => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        "probing" => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        "stale" => Style::default()
            .fg(Color::Rgb(255, 165, 0))
            .add_modifier(Modifier::BOLD),
        "failed" | "blocked" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    };
    Line::from(vec![
        Span::styled(
            id.to_owned(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(status.to_owned(), status_style),
        Span::raw(" | "),
        Span::styled(title.to_owned(), Style::default().fg(Color::Gray)),
    ])
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let n = app.logs.len();
    let lines: Vec<Line<'static>> = if app.logs.is_empty() {
        vec![Line::from(Span::styled(
            "(empty · r refresh · c clear)".to_owned(),
            Style::default().fg(Color::Cyan),
        ))]
    } else {
        app.logs
            .iter()
            .rev()
            .take(40)
            .rev()
            .map(|line| style_log_line(line))
            .collect()
    };
    let p = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(ratatui::widgets::Padding::new(0, 0, 1, 0))
            .title(Line::from(vec![
                panel_title("Logs"),
                Span::styled(format!(" ({n})"), Style::default().fg(Color::Cyan)),
            ])),
    );
    f.render_widget(p, area);
}

fn style_log_line(line: &str) -> Line<'static> {
    let (ts, rest) = match line.split_once(' ') {
        Some((t, r)) if t.len() == 8 && t.chars().filter(|c| *c == ':').count() == 2 => (t, r),
        _ => {
            return Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(Color::White),
            ));
        }
    };
    let lower = rest.to_ascii_lowercase();
    let msg_style =
        if lower.contains("error") || lower.contains("failed") || lower.contains("fail:") {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
    Line::from(vec![
        Span::styled(ts.to_owned(), Style::default().fg(Color::Gray)),
        Span::raw(" "),
        Span::styled(rest.to_owned(), msg_style),
    ])
}

fn draw_panel(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.screen {
        Screen::Overview => "Overview",
        Screen::Ssh => "SSH",
        Screen::Cli => "CLI",
        Screen::Api => "API",
        Screen::Mcp => "MCP",
        Screen::Reboot => "Reboot",
        Screen::Setup | Screen::Logs => "",
    };
    let p = Paragraph::new(app.panel_lines.clone())
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .padding(ratatui::widgets::Padding::new(0, 0, 1, 0))
                .title(Line::from(panel_title(title))),
        );
    f.render_widget(p, area);
}
