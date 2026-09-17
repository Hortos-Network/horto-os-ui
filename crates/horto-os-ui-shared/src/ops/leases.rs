use crate::context::HostContext;
use crate::error::{HortoError, Result};
use crate::kits::fs as fs_kit;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseEntry {
    pub hostname: String,
    pub ip: String,
    pub mac: String,
    pub expires: String,
}

pub fn read_leases(lease_file: &Path, leases_json: &Path) -> Vec<LeaseEntry> {
    if lease_file.is_file() {
        if let Ok(text) = std::fs::read_to_string(lease_file) {
            return parse_dnsmasq_leases(&text);
        }
    }
    if leases_json.is_file() {
        if let Ok(text) = std::fs::read_to_string(leases_json) {
            if let Ok(list) = serde_json::from_str::<Vec<LeaseEntry>>(&text) {
                return list;
            }
        }
    }
    Vec::new()
}

pub fn parse_dnsmasq_leases(text: &str) -> Vec<LeaseEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        out.push(LeaseEntry {
            expires: parts[0].to_string(),
            mac: parts[1].to_string(),
            ip: parts[2].to_string(),
            hostname: parts[3].to_string(),
        });
    }
    out
}

/// Port of export_dhcp_leases.sh: write leases.json + leases.html under docker assets.
pub fn export_dhcp_leases(ctx: &mut HostContext) -> Result<()> {
    let lease_file = ctx.paths.lease_file.clone();
    let out_dir = ctx.paths.docker_assets();
    let out_json = ctx.paths.leases_json();
    let out_html = ctx.paths.leases_html();

    fs_kit::ensure_dir(ctx, &out_dir)?;

    if lease_file.is_file() && out_html.is_file() {
        if let (Ok(lm), Ok(hm)) = (
            lease_file.metadata().and_then(|m| m.modified()),
            out_html.metadata().and_then(|m| m.modified()),
        ) {
            if lm <= hm {
                ctx.log("Leases unchanged since last export; skipping.");
                return Ok(());
            }
        }
    }

    if !lease_file.is_file() {
        let empty_json = "[]\n";
        let empty_html = "<html><body><p>No DHCP leases found.</p></body></html>\n";
        fs_kit::write_file(ctx, &out_json, empty_json.as_bytes())?;
        fs_kit::write_file(ctx, &out_html, empty_html.as_bytes())?;
        ctx.log("Lease file not found, wrote empty output.");
        return Ok(());
    }

    if ctx.is_dry_run() {
        ctx.plan_action(format!(
            "export leases from {} to {} and {}",
            lease_file.display(),
            out_json.display(),
            out_html.display()
        ));
        return Ok(());
    }

    let text = std::fs::read_to_string(&lease_file)
        .map_err(|e| HortoError::msg(format!("read leases: {e}")))?;
    let leases = parse_dnsmasq_leases(&text);
    let json = serde_json::to_string_pretty(&leases)?;
    let html = render_leases_html(&leases);
    std::fs::write(&out_json, json)?;
    std::fs::write(&out_html, html)?;
    let _ = SystemTime::now();
    ctx.log(format!(
        "DHCP leases exported to {} and {}",
        out_json.display(),
        out_html.display()
    ));
    Ok(())
}

fn render_leases_html(leases: &[LeaseEntry]) -> String {
    let mut out = String::from(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
body{font-family:ui-sans-serif,sans-serif;margin:0;padding:0;background:transparent;color:inherit}
table{width:100%;border-collapse:collapse;font-size:.85rem}
th,td{text-align:left;padding:6px 8px;border-bottom:1px solid rgba(255,255,255,.12)}
th{font-weight:600;opacity:.7;text-transform:uppercase;font-size:.7rem;letter-spacing:.5px}
tr:hover{background:rgba(255,255,255,.05)}
</style>
</head>
<body>
<table>
<tr><th>Hostname</th><th>IP</th><th>MAC</th></tr>
"#,
    );
    for l in leases {
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td style=\"font-family:monospace;font-size:.8rem\">{}</td></tr>\n",
            html_escape(&l.hostname),
            html_escape(&l.ip),
            html_escape(&l.mac)
        ));
    }
    out.push_str("</table>\n</body>\n</html>\n");
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
