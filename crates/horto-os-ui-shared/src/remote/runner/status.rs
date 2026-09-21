//! Parse and fetch remote setup status and doctor reports.

use super::super::process::ProcessRunner;
use super::options::RemoteOptions;
use super::probe::remote_box_snapshot;
use crate::error::Result;

/// Parse the first JSON object/array from remote captured output.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when no JSON is found or deserialization fails.
pub fn parse_remote_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    let start = raw.find('{').or_else(|| raw.find('[')).ok_or_else(|| {
        crate::error::HortoError::msg("remote output had no JSON object or array")
    })?;
    serde_json::from_str(raw[start..].trim())
        .map_err(|e| crate::error::HortoError::msg(format!("parse remote JSON: {e}")))
}

/// Parse human `setup status` lines from older box agents (no `--json`).
pub fn parse_setup_status_text(raw: &str) -> Result<crate::ops::status::SetupStatusReport> {
    use crate::ops::status::{SetupStatusReport, StepStatusRow};
    let mut kind = "full".to_owned();
    let mut steps = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Setup kind:") {
            rest.trim().clone_into(&mut kind);
            continue;
        }
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((status_part, after_status)) = rest.split_once(']') else {
            continue;
        };
        let status = status_part.trim().to_owned();
        let after_status = after_status.trim();
        let Some((id, after_id)) = after_status.split_once(" - ") else {
            continue;
        };
        let (title, ver_flags) = after_id
            .rfind(" (v")
            .map_or((after_id, ""), |i| (&after_id[..i], &after_id[i..]));
        let step_version = ver_flags
            .trim_start_matches(" (v")
            .split(')')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        steps.push(StepStatusRow {
            id: id.trim().to_owned(),
            title: title.trim().to_owned(),
            status,
            step_version,
            destructive: ver_flags.contains("[destructive]"),
            needs_reboot_after: ver_flags.contains("[reboot]"),
        });
    }
    if steps.is_empty() {
        return Err(crate::error::HortoError::msg(
            "remote setup status produced no step lines",
        ));
    }
    Ok(SetupStatusReport { kind, steps })
}

/// Fetch setup step status from the box (`setup status`, JSON when supported).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or status output cannot be parsed.
pub fn remote_setup_status(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
    full: bool,
) -> Result<crate::ops::status::SetupStatusReport> {
    let snap = remote_box_snapshot(runner, opts, full)?;
    snap.setup.ok_or_else(|| {
        crate::error::HortoError::msg(
            "box CLI missing or outdated; sync CLI (s0) before setup status",
        )
    })
}

/// Fetch doctor JSON from the box.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when SSH fails or the remote JSON cannot be parsed.
pub fn remote_doctor(
    runner: &dyn ProcessRunner,
    opts: &RemoteOptions,
) -> Result<crate::ops::doctor::DoctorReport> {
    let snap = remote_box_snapshot(runner, opts, true)?;
    snap.doctor.ok_or_else(|| {
        crate::error::HortoError::msg("box CLI missing or outdated; sync CLI (s0) before doctor")
    })
}
