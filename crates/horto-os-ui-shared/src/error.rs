//! Typed setup/ops errors for the Horto engine.
//!
//! # Layering (ITC habit)
//!
//! | Layer | Crate | Tool |
//! | ----- | ----- | ---- |
//! | Library / step API | `horto-os-ui-shared` | `thiserror` via [`HortoError`] |
//! | Process binaries (CLI, TUI, API, KPI) | each binary | `anyhow` at `main`; `?` maps [`HortoError`] |
//! | Diagnostics | shared emits; CLI/API/KPI/desktop call [`crate::init_tracing`] | `tracing` only (stderr via subscriber); TUI uses [`crate::HostContext::logs`] |
//!
//! Do **not** put `thiserror` on every surface crate. Surfaces that only talk HTTP
//! (KPI, web) never see [`HortoError`]. Surfaces that call the engine convert with `?`.

use thiserror::Error;

/// Crate-wide result alias for the setup engine.
pub type Result<T> = std::result::Result<T, HortoError>;

/// Failures from steps, kits, resume, and host ops.
#[derive(Debug, Error)]
pub enum HortoError {
    /// Free-form message when no tighter variant fits yet.
    #[error("{0}")]
    Message(String),

    /// Apply mode needs root (sudo).
    #[error("root required for apply mode; re-run with sudo")]
    RootRequired,

    /// Step id is not registered.
    #[error("unknown step id: {0}")]
    UnknownStep(String),

    /// Step id exists but is not in the selected Full/Minimal pipeline.
    #[error("step {step} is not in the {kind} pipeline")]
    NotInPipeline {
        /// Step id requested.
        step: String,
        /// Pipeline kind (`full` / `minimal`).
        kind: String,
    },

    /// Required dependency steps are incomplete.
    #[error("step {0} depends on incomplete step(s): {1}")]
    MissingDependency(String, String),

    /// Embedded asset path missing from the binary.
    #[error("missing embedded asset: {0}")]
    EmbedMissing(String),

    /// External process failed (apt, systemctl, docker, iptables, …).
    #[error("command failed: {program}: {detail}")]
    CommandFailed {
        /// Program name or short label.
        program: String,
        /// Status / stderr summary.
        detail: String,
    },

    /// Filesystem IO.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON encode/decode.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl HortoError {
    /// Build a [`Message`](Self::Message) error.
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }

    /// External command failure helper.
    pub fn command(program: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::CommandFailed {
            program: program.into(),
            detail: detail.into(),
        }
    }

    /// Suggested process exit code for CLI surfaces.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::RootRequired => 77,
            Self::UnknownStep(_) | Self::NotInPipeline { .. } => 2,
            _ => 1,
        }
    }
}
