use std::collections::BTreeMap;

/// Replace `{{VAR}}` placeholders using the provided map.
///
/// # Examples
///
/// ```
/// use horto_os_ui_shared::kits::template;
/// use std::collections::BTreeMap;
///
/// let mut vars = BTreeMap::new();
/// vars.insert("MY_HOSTNAME".into(), "box1".into());
/// assert_eq!(template::render("host={{MY_HOSTNAME}}", &vars), "host=box1");
/// ```
pub fn render(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        let needle = format!("{{{{{key}}}}}");
        out = out.replace(&needle, value);
    }
    out
}

/// Return true when `{{UPPER_SNAKE}}` placeholders remain.
///
/// # Examples
///
/// ```
/// use horto_os_ui_shared::kits::template;
///
/// assert!(template::has_unreplaced_placeholders("x={{FOO}}"));
/// assert!(!template::has_unreplaced_placeholders("x=box1"));
/// ```
#[must_use]
pub fn has_unreplaced_placeholders(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = text[i + 2..].find("}}") {
                let name = &text[i + 2..i + 2 + end];
                if name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    && !name.is_empty()
                {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn render_replaces_vars() {
        let mut vars = BTreeMap::new();
        vars.insert("MY_HOSTNAME".into(), "box1".into());
        let out = render("host={{MY_HOSTNAME}}", &vars);
        assert_eq!(out, "host=box1");
    }

    #[test]
    fn detects_placeholders() {
        assert!(has_unreplaced_placeholders("x={{FOO}} y"));
        assert!(!has_unreplaced_placeholders("x=box1"));
    }
}
