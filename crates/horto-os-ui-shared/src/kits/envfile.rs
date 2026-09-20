//! `KEY=value` env file parse / write helpers.
//!
//! # Examples
//!
//! ```
//! use horto_os_ui_shared::kits::envfile;
//!
//! let map = envfile::parse("FOO=bar\n# c\nBAZ=\"x y\"\n");
//! assert_eq!(map.get("FOO").map(String::as_str), Some("bar"));
//! assert_eq!(map.get("BAZ").map(String::as_str), Some("x y"));
//! ```

use crate::error::{HortoError, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Parse KEY=value / KEY="value" lines into an ordered map.
#[must_use]
pub fn parse(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = unquote(raw.trim());
        map.insert(key, value);
    }
    map
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }
    let quote = bytes[0];
    if quote == b'"' || quote == b'\'' {
        let mut out = String::new();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if bytes[i] == quote {
                return out;
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        return out;
    }
    s.split_once('#')
        .map(|(v, _)| v.trim())
        .unwrap_or(s)
        .to_string()
}

fn escape_double(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Write KEY="value" lines.
pub fn write(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    let mut out = String::new();
    for (k, v) in map {
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&escape_double(v));
        out.push_str("\"\n");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, out)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<BTreeMap<String, String>> {
    let content = fs::read_to_string(path)
        .map_err(|e| HortoError::msg(format!("cannot read {}: {e}", path.display())))?;
    Ok(parse(&content))
}

pub fn set_key(map: &mut BTreeMap<String, String>, key: &str, value: impl Into<String>) {
    map.insert(key.to_string(), value.into());
}

pub fn require_keys(map: &BTreeMap<String, String>, keys: &[&str]) -> Result<()> {
    for key in keys {
        match map.get(*key) {
            Some(v) if !v.is_empty() => {}
            _ => {
                return Err(HortoError::msg(format!(
                    "required variable {key} is empty or missing"
                )));
            }
        }
    }
    Ok(())
}

/// True when `WIFI_INTERFACE` names a real WiFi AP iface (not `none` / empty / `n`).
#[must_use]
pub fn wifi_ap_enabled(map: &BTreeMap<String, String>) -> bool {
    map.get("WIFI_INTERFACE")
        .is_some_and(|v| wifi_iface_enabled(v))
}

/// True when the WiFi interface value should drive hostapd / SSID prompts.
#[must_use]
pub fn wifi_iface_enabled(raw: &str) -> bool {
    let t = raw.trim().to_ascii_lowercase();
    !(t.is_empty() || matches!(t.as_str(), "none" | "-" | "n" | "no"))
}

/// Normalize optional WiFi iface: disabled answers become `none`.
#[must_use]
pub fn normalize_wifi_iface(raw: &str) -> String {
    let trimmed = raw.trim();
    if wifi_iface_enabled(trimmed) {
        trimmed.to_string()
    } else {
        "none".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parse_inline_comments() {
        let map = parse("NPU_TYPE=\"rk3588\" # comment\nOS_TYPE=debian # other\n");
        assert_eq!(map.get("NPU_TYPE").map(String::as_str), Some("rk3588"));
        assert_eq!(map.get("OS_TYPE").map(String::as_str), Some("debian"));
    }

    #[test]
    fn write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vars.env");
        let mut map = BTreeMap::new();
        map.insert("A".into(), "one".into());
        map.insert("B".into(), "has \"quote\"".into());
        write(&path, &map).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.get("A").map(String::as_str), Some("one"));
        assert_eq!(loaded.get("B").map(String::as_str), Some("has \"quote\""));
    }

    #[test]
    fn wifi_iface_none_disables_ap() {
        assert!(!wifi_iface_enabled("none"));
        assert!(!wifi_iface_enabled("NONE"));
        assert!(!wifi_iface_enabled("n"));
        assert!(!wifi_iface_enabled(""));
        assert!(wifi_iface_enabled("wlan0"));
        assert!(wifi_iface_enabled("wlx00aabb"));
        assert_eq!(normalize_wifi_iface("n"), "none");
        assert_eq!(normalize_wifi_iface(" wlan0 "), "wlan0");

        let mut map = BTreeMap::new();
        map.insert("WIFI_INTERFACE".into(), "none".into());
        assert!(!wifi_ap_enabled(&map));
        map.insert("WIFI_INTERFACE".into(), "wlan0".into());
        assert!(wifi_ap_enabled(&map));
    }
}
