use crate::error::Result;
use crate::paths::HostPaths;
use crate::step::Step;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResumeState {
    pub steps: Vec<ResumeStepRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeStepRecord {
    pub id: String,
    pub step_version: u32,
    pub completed_at: DateTime<Utc>,
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Done,
    Stale,
    Failed,
}

pub fn load(path: &Path) -> Result<ResumeState> {
    if !path.exists() {
        return Ok(ResumeState::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

pub fn save(path: &Path, state: &ResumeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(state)?;
    fs::write(path, text)?;
    Ok(())
}

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

pub fn status_for(state: &ResumeState, step: &dyn Step) -> StepStatus {
    match state.steps.iter().rev().find(|r| r.id == step.id()) {
        Some(r) if r.ok && r.step_version >= step.step_version() => StepStatus::Done,
        Some(r) if r.ok && r.step_version < step.step_version() => StepStatus::Stale,
        Some(r) if !r.ok => StepStatus::Failed,
        _ => StepStatus::Pending,
    }
}

/// True when recorded completion is older than the binary's step_version.
pub fn is_stale(recorded_version: u32, binary_version: u32) -> bool {
    binary_version > recorded_version
}

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
