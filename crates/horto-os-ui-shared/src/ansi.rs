//! Strip terminal escape sequences from remote setup output.

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

#[cfg(test)]
mod tests {
    use super::strip_ansi;

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
    fn strip_ansi_drops_osc_and_bare_esc() {
        let osc = "pre\u{1b}]0;title\u{7}post";
        assert_eq!(strip_ansi(osc), "prepost");
        let bare = "a\u{1b}Xb";
        assert_eq!(strip_ansi(bare), "ab");
        assert_eq!(strip_ansi("tail\u{1b}"), "tail");
    }
}
