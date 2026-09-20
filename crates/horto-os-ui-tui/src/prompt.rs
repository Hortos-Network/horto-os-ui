//! Ratatui modals: y/N confirm, free-text, and masked secret input.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Outcome of a confirm modal key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    /// Enter / y
    Yes,
    /// Esc / n
    No,
    /// Key ignored
    Ignore,
}

/// Handle y/N / Enter / Esc for a confirm overlay.
#[must_use]
pub fn confirm_key(key: KeyEvent) -> ConfirmResult {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmResult::Yes,
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => ConfirmResult::No,
        _ => ConfirmResult::Ignore,
    }
}

/// Free-text modal (non-secret). Global shortcuts must not fire while open.
#[derive(Debug, Clone)]
pub struct TextInput {
    title: String,
    buffer: String,
}

/// Masked secret modal (sudo password). Same keys as [`TextInput`]; display is `*`.
#[derive(Debug, Clone)]
pub struct SecretInput {
    title: String,
    buffer: String,
}

/// Result of handling a key in [`TextInput`] / [`SecretInput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputResult {
    /// Still editing
    Continue,
    /// Enter: submit current buffer (trimmed by caller if desired)
    Submit(String),
    /// Esc: cancel
    Cancel,
}

impl TextInput {
    /// Create a text modal with an optional initial value.
    #[must_use]
    pub fn new(title: impl Into<String>, initial: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            buffer: initial.into(),
        }
    }

    /// Apply a key. Printable chars edit; Backspace deletes; Enter/Esc finish.
    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputResult {
        edit_buffer(&mut self.buffer, key)
    }
}

impl SecretInput {
    /// Create a masked secret modal (empty buffer).
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            buffer: String::new(),
        }
    }

    /// Apply a key (same as text; buffer is never shown).
    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputResult {
        edit_buffer(&mut self.buffer, key)
    }
}

fn edit_buffer(buffer: &mut String, key: KeyEvent) -> TextInputResult {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return TextInputResult::Continue;
    }
    match key.code {
        KeyCode::Enter => TextInputResult::Submit(std::mem::take(buffer)),
        KeyCode::Esc => TextInputResult::Cancel,
        KeyCode::Backspace => {
            buffer.pop();
            TextInputResult::Continue
        }
        KeyCode::Char(c) if !c.is_control() => {
            buffer.push(c);
            TextInputResult::Continue
        }
        _ => TextInputResult::Continue,
    }
}

fn modal_block(title: &str, border: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_owned())
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .border_style(
            Style::default()
                .fg(border)
                .bg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
}

/// Draw a compact centered confirm dialog (opaque so the panel behind cannot bleed).
pub fn draw_confirm(f: &mut Frame, title: &str, body: &str) {
    let area = centered_fixed(64, 7, f.area());
    f.render_widget(Clear, area);
    let text = format!("{body}\n\nEnter/y confirm · Esc/n cancel");
    let p = Paragraph::new(text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(modal_block(title, Color::Yellow));
    f.render_widget(p, area);
}

/// Draw a centered text-input dialog (opaque).
pub fn draw_text_input(f: &mut Frame, input: &TextInput) {
    let area = centered_fixed(70, 8, f.area());
    f.render_widget(Clear, area);
    let text = format!("{}\n\nEnter submit · Esc cancel", input.buffer);
    let p = Paragraph::new(text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(modal_block(&input.title, Color::Cyan));
    f.render_widget(p, area);
}

/// Draw a centered masked secret dialog (opaque).
pub fn draw_secret_input(f: &mut Frame, input: &SecretInput) {
    let area = centered_fixed(56, 7, f.area());
    f.render_widget(Clear, area);
    let masked = if input.buffer.is_empty() {
        " ".to_owned()
    } else {
        std::iter::repeat_n('*', input.buffer.chars().count()).collect()
    };
    let text = format!("{masked}\n\nEnter submit · Esc cancel");
    let p = Paragraph::new(text)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .block(modal_block(&input.title, Color::Yellow));
    f.render_widget(p, area);
}

fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn confirm_yes_no() {
        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let n = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(confirm_key(y), ConfirmResult::Yes);
        assert_eq!(confirm_key(n), ConfirmResult::No);
    }

    #[test]
    fn text_input_type_backspace_submit_cancel() {
        let mut t = TextInput::new("Host", "ho");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(t.buffer, "hor");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(t.buffer, "ho");
        assert_eq!(
            t.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            TextInputResult::Submit("ho".into())
        );
        let mut t2 = TextInput::new("Host", "x");
        assert_eq!(
            t2.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            TextInputResult::Cancel
        );
    }

    #[test]
    fn text_input_ignores_control_chars_as_shortcuts() {
        let mut t = TextInput::new("Host", String::new());
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(t.handle_key(ctrl_c), TextInputResult::Continue);
        assert!(t.buffer.is_empty());
    }

    #[test]
    fn secret_input_masks_and_submits() {
        let mut s = SecretInput::new("Sudo password");
        assert_eq!(
            s.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(
            s.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE)),
            TextInputResult::Continue
        );
        assert_eq!(s.buffer, "ab");
        assert_eq!(
            s.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            TextInputResult::Submit("ab".into())
        );
        assert!(s.buffer.is_empty());
    }
}
