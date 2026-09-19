//! Docker helpers for status listing and compose rebuild.
//!
//! reference: horto-os/scripts/docker_rebuild.sh

use crate::error::{HortoError, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// One running container row from `docker ps`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerInfo {
    /// Short or full container id.
    pub id: String,
    /// Comma-separated names.
    pub names: String,
    /// Image reference.
    pub image: String,
    /// Status column from `docker ps`.
    pub status: String,
    /// Ports column (may be empty).
    pub ports: String,
    /// Compose project / Dockge stack name when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// Optional catalog blurb for known Horto images / names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Whether `docker` is on `PATH`.
#[must_use]
pub fn docker_available() -> bool {
    which::which("docker").is_ok()
}

/// Parse `docker ps --format` tab-separated lines into [`ContainerInfo`] rows.
///
/// Columns: id, names, image, status, ports (optional), compose project (optional).
///
/// # Examples
///
/// ```
/// use horto_os_ui_shared::kits::docker::parse_docker_ps_lines;
///
/// let rows = parse_docker_ps_lines("abc\tweb\tnginx:latest\tUp 1h\t80/tcp\thomepage\n");
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows[0].names, "web");
/// assert_eq!(rows[0].stack.as_deref(), Some("homepage"));
/// ```
#[must_use]
pub fn parse_docker_ps_lines(text: &str) -> Vec<ContainerInfo> {
    let mut list = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 4 {
            let names = parts[1].to_string();
            let project = parts.get(5).map(|s| s.trim()).unwrap_or("");
            let stack = resolve_stack(&names, project);
            list.push(ContainerInfo {
                id: parts[0].to_string(),
                names,
                image: parts[2].to_string(),
                status: parts[3].to_string(),
                ports: parts.get(4).unwrap_or(&"").to_string(),
                stack,
                description: None,
            });
        }
    }
    list
}

/// Dockge stack id: compose project label, else primary container name.
#[must_use]
pub fn resolve_stack(names: &str, compose_project: &str) -> Option<String> {
    let project = compose_project.trim();
    if !project.is_empty() {
        return Some(project.to_owned());
    }
    names
        .split(',')
        .map(str::trim)
        .map(|n| n.trim_start_matches('/'))
        .find(|n| !n.is_empty())
        .map(str::to_owned)
}

/// List running containers, or an empty list when docker is missing / fails.
///
/// # Errors
///
/// Currently always returns `Ok`; IO failures degrade to an empty list.
pub fn list_containers() -> Result<Vec<ContainerInfo>> {
    if !docker_available() {
        return Ok(Vec::new());
    }
    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}\t{{.Label \"com.docker.compose.project\"}}",
        ])
        .output();
    let Ok(output) = output else {
        return Ok(Vec::new());
    };
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_docker_ps_lines(&text))
}

/// Rebuild and recreate one compose project in `dir` (`build --no-cache` then `up -d`).
///
/// # Errors
///
/// Returns [`HortoError::Message`] when docker is missing, or [`HortoError::CommandFailed`]
/// when compose fails.
pub fn docker_rebuild(dir: &std::path::Path) -> Result<()> {
    if !docker_available() {
        return Err(HortoError::msg("docker not found on PATH"));
    }
    let status = Command::new("docker")
        .args(["compose", "build", "--no-cache"])
        .current_dir(dir)
        .status()
        .map_err(|e| HortoError::command("docker", format!("compose build: {e}")))?;
    if !status.success() {
        return Err(HortoError::command(
            "docker",
            "compose build --no-cache failed",
        ));
    }
    let status = Command::new("docker")
        .args(["compose", "up", "-d"])
        .current_dir(dir)
        .status()
        .map_err(|e| HortoError::command("docker", format!("compose up: {e}")))?;
    if !status.success() {
        return Err(HortoError::command("docker", "compose up -d failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skips_short_lines() {
        assert!(parse_docker_ps_lines("only\ttwo\n").is_empty());
        let rows = parse_docker_ps_lines("id\tname\timg\tup\n");
        assert_eq!(rows[0].ports, "");
        assert_eq!(rows[0].stack.as_deref(), Some("name"));
    }

    #[test]
    fn resolve_prefers_compose_project() {
        assert_eq!(
            resolve_stack("web", "homepage").as_deref(),
            Some("homepage")
        );
        assert_eq!(
            resolve_stack("/piper,/piper-1", "").as_deref(),
            Some("piper")
        );
    }

    #[test]
    fn resolve_stack_none_when_no_source() {
        assert_eq!(resolve_stack("", ""), None);
        assert_eq!(resolve_stack(",,,", "").as_deref(), None);
    }

    #[test]
    fn resolve_stack_trims_whitespace_project() {
        assert_eq!(
            resolve_stack("web", "  homepage  ").as_deref(),
            Some("homepage")
        );
    }

    #[test]
    fn parse_docker_ps_multiple_rows_and_project_column() {
        let text = "id1\ta\tnginx\tUp\t80/tcp\thomepage\nid2\tb\tredis\tUp\t\n";
        let rows = parse_docker_ps_lines(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].stack.as_deref(), Some("homepage"));
        // Empty project falls back to container name.
        assert_eq!(rows[1].stack.as_deref(), Some("b"));
        assert_eq!(rows[1].ports, "");
    }

    #[test]
    fn parse_docker_ps_skips_headers_and_blank() {
        assert!(parse_docker_ps_lines("").is_empty());
        assert!(parse_docker_ps_lines("\n\n").is_empty());
    }

    #[test]
    fn docker_available_returns_bool() {
        // Just exercise the code path; the value depends on host PATH.
        let _ = docker_available();
    }

    #[test]
    fn list_containers_never_fails_on_absent_daemon() {
        // Regardless of whether docker is installed, list_containers is Ok.
        let list = list_containers().unwrap();
        // We cannot assert length; on a dev host with docker running it may be non-empty.
        // Any Vec<ContainerInfo> is fine.
        let _ = list;
    }

    #[test]
    fn container_info_serializes_stack_and_description() {
        let row = ContainerInfo {
            id: "abc".into(),
            names: "web".into(),
            image: "nginx".into(),
            status: "Up".into(),
            ports: "80/tcp".into(),
            stack: Some("homepage".into()),
            description: Some("blurb".into()),
        };
        let json = serde_json::to_string(&row).unwrap();
        assert!(json.contains("\"stack\":\"homepage\""));
        assert!(json.contains("\"description\":\"blurb\""));
        let back: ContainerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn container_info_omits_optional_fields_when_none() {
        let row = ContainerInfo {
            id: "abc".into(),
            names: "solo".into(),
            image: "img".into(),
            status: "Up".into(),
            ports: String::new(),
            stack: None,
            description: None,
        };
        let json = serde_json::to_string(&row).unwrap();
        assert!(!json.contains("\"stack\""));
        assert!(!json.contains("\"description\""));
    }

    #[test]
    fn docker_rebuild_returns_error_without_valid_compose() {
        // Missing docker or empty compose dir both error; always exercise the call.
        let tmp = tempfile::TempDir::new().unwrap();
        let err = docker_rebuild(tmp.path()).unwrap_err();
        let msg = err.to_string().to_ascii_lowercase();
        let has_docker = msg.contains("docker");
        let has_compose = msg.contains("compose");
        assert!(has_docker | has_compose, "unexpected error: {msg}");
    }
}
