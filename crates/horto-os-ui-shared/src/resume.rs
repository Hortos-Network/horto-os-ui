//! Persist and query per-step completion for setup resume.

use crate::error::Result;
use crate::paths::HostPaths;
use crate::step::Step;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Serialized resume file: ordered step completion records.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResumeState {
    /// Completion / failure records (latest per id wins for [`status_for`]).
    pub steps: Vec<ResumeStepRecord>,
}

/// One recorded attempt for a step id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeStepRecord {
    /// Step id (`s1`, `d1`, …).
    pub id: String,
    /// [`Step::step_version`] at record time.
    pub step_version: u32,
    /// UTC timestamp when the record was written.
    pub completed_at: DateTime<Utc>,
    /// `true` on success, `false` on recorded failure.
    pub ok: bool,
}

/// Derived status of a registered step against [`ResumeState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// No matching record (or only superseded entries).
    Pending,
    /// Last record is ok and `step_version` is current or newer than the binary's.
    Done,
    /// Last record is ok but recorded version is older than the binary's.
    Stale,
    /// Last record has `ok: false`.
    Failed,
}

/// Load resume JSON from `path`, or an empty state when the file is missing.
///
/// # Errors
///
/// Returns [`HortoError::Io`](crate::error::HortoError::Io) on read failure, or
/// [`HortoError::Json`](crate::error::HortoError::Json) when the file is not valid JSON.
pub fn load(path: &Path) -> Result<ResumeState> {
    if !path.exists() {
        return Ok(ResumeState::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Write resume JSON (pretty) to `path`, creating parent directories as needed.
///
/// # Errors
///
/// Returns [`HortoError::Io`](crate::error::HortoError::Io) on directory/create/write failure, or
/// [`HortoError::Json`](crate::error::HortoError::Json) if serialization fails.
pub fn save(path: &Path, state: &ResumeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(state)?;
    fs::write(path, text)?;
    Ok(())
}

/// Replace any prior record for `step` with a successful completion and save.
///
/// # Errors
///
/// Propagates [`load`] / [`save`] errors (`Io`, `Json`).
pub fn record_ok(paths: &HostPaths, step: &dyn Step) -> Result<()> {
    let path = paths.resume_file();
    let mut state = load(&path)?;
    state.steps.retain(|r| r.id != step.id());
    state.steps.push(ResumeStepRecord {
        id: step.id().to_string(),
        step_version: step.step_version(),
        completed_at: Utc::now(),
        ok: true,
    });
    save(&path, &state)
}

/// Compute [`StepStatus`] from the latest matching record in `state`.
#[must_use]
pub fn status_for(state: &ResumeState, step: &dyn Step) -> StepStatus {
    match state.steps.iter().rev().find(|r| r.id == step.id()) {
        Some(r) if r.ok && r.step_version >= step.step_version() => StepStatus::Done,
        Some(r) if r.ok && r.step_version < step.step_version() => StepStatus::Stale,
        Some(r) if !r.ok => StepStatus::Failed,
        _ => StepStatus::Pending,
    }
}

/// True when recorded completion is older than the binary's `step_version`.
#[must_use]
pub const fn is_stale(recorded_version: u32, binary_version: u32) -> bool {
    binary_version > recorded_version
}

/// Replace any prior record for `step_id` with a failure and save.
///
/// On load failure, starts from an empty [`ResumeState`] instead of aborting.
///
/// # Errors
///
/// Propagates [`save`] errors (`Io`, `Json`).
pub fn mark_failed(paths: &HostPaths, step_id: &str, step_version: u32) -> Result<()> {
    let path = paths.resume_file();
    let mut state = load(&path).unwrap_or_default();
    state.steps.retain(|r| r.id != step_id);
    state.steps.push(ResumeStepRecord {
        id: step_id.to_string(),
        step_version,
        completed_at: Utc::now(),
        ok: false,
    });
    save(&path, &state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_when_binary_newer() {
        assert!(is_stale(1, 2));
        assert!(!is_stale(2, 2));
        assert!(!is_stale(3, 2));
    }
}
