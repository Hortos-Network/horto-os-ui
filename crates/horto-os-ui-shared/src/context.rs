//! Host execution context for setup steps: apply vs dry-run, paths, logs, prompts.

use crate::paths::HostPaths;
use crate::remote::EcosystemInstallChoice;
use crate::stack_opts::StackOpts;
use std::collections::HashMap;

/// Whether steps mutate the host (`Apply`) or only record planned actions (`DryRun`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Record planned actions; kits skip mutating side effects.
    DryRun,
    /// Perform real filesystem / package / systemd changes (needs root).
    Apply,
}

/// One planned side effect recorded during dry-run (or planning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    /// Human-readable summary of the intended action.
    pub summary: String,
}

impl PlannedAction {
    /// Build a planned action from a summary string.
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }
}

/// Optional interactive prompts. Non-interactive defaults skip prompts.
pub trait PromptsProvider: Send {
    /// Ask for a free-form value; return `default` when the operator accepts empty input.
    fn prompt(&mut self, label: &str, default: &str) -> String;
    /// Yes/no confirm; return `default_yes` when input is empty or unrecognized.
    fn confirm(&mut self, question: &str, default_yes: bool) -> bool;
}

/// Always returns defaults / no for confirms.
#[derive(Debug, Default)]
pub struct NonInteractivePrompts;

impl PromptsProvider for NonInteractivePrompts {
    fn prompt(&mut self, _label: &str, default: &str) -> String {
        default.to_string()
    }

    fn confirm(&mut self, _question: &str, default_yes: bool) -> bool {
        default_yes
    }
}

/// Stdin/stdout prompts for interactive CLI/TUI use.
#[derive(Debug, Default)]
pub struct StdioPrompts;

impl PromptsProvider for StdioPrompts {
    fn prompt(&mut self, label: &str, default: &str) -> String {
        use std::io::{self, IsTerminal, Write};
        // Piped SSH / Capture: do not spam prompt labels into captured stderr.
        if !io::stdin().is_terminal() {
            return default.to_string();
        }
        eprint!("{label} [{default}]: ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return default.to_string();
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            default.to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn confirm(&mut self, question: &str, default_yes: bool) -> bool {
        use std::io::{self, IsTerminal, Write};
        if !io::stdin().is_terminal() {
            return default_yes;
        }
        let hint = if default_yes { "Y/n" } else { "y/N" };
        eprint!("{question} [{hint}]: ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return default_yes;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => true,
            "n" | "no" => false,
            _ => default_yes,
        }
    }
}

/// Mutable per-run state passed into steps and kits.
pub struct HostContext {
    /// Dry-run vs apply.
    pub mode: ApplyMode,
    /// Host filesystem layout for this run.
    pub paths: HostPaths,
    /// Selected Full / Minimal pipeline.
    pub setup_kind: crate::pipeline::SetupKind,
    /// When true, Piper-related install paths are skipped (also when Piper stack is not opted).
    pub skip_piper: bool,
    /// Optional Docker stacks for `d3` (Homepage is always `d2`).
    pub stack_opts: StackOpts,
    /// Status-api / MCP choice used for payload install and `service_links` filtering.
    pub ecosystem: EcosystemInstallChoice,
    /// When true, NAT-related confirms auto-accept (also set via `HORTO_APPLY_NAT`).
    pub apply_nat: bool,
    /// Accumulated log lines for TUI / API surfaces (also mirrored to `tracing`).
    pub logs: Vec<String>,
    /// Planned actions collected during dry-run / `plan`.
    pub planned: Vec<PlannedAction>,
    prompts: Option<Box<dyn PromptsProvider>>,
    /// Pre-filled answers keyed by prompt label (tests / automation).
    pub prompt_answers: HashMap<String, String>,
}

impl HostContext {
    /// Create a context with default [`HostPaths`] and empty logs / planned actions.
    ///
    /// `apply_nat` is initialized from `HORTO_APPLY_NAT=1` / `true` when set.
    #[must_use]
    pub fn new(mode: ApplyMode, kind: crate::pipeline::SetupKind) -> Self {
        Self {
            mode,
            paths: HostPaths::default(),
            setup_kind: kind,
            skip_piper: false,
            stack_opts: StackOpts::none(),
            ecosystem: EcosystemInstallChoice::default(),
            apply_nat: std::env::var("HORTO_APPLY_NAT")
                .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            logs: Vec::new(),
            planned: Vec::new(),
            prompts: None,
            prompt_answers: HashMap::new(),
        }
    }

    /// True when Piper model download should be skipped (`--skip-piper` or Piper not in stacks).
    #[must_use]
    pub const fn skip_piper_download(&self) -> bool {
        self.skip_piper || !self.stack_opts.piper
    }

    /// Replace host paths (tests and alternate roots).
    #[must_use]
    pub fn with_paths(mut self, paths: HostPaths) -> Self {
        self.paths = paths;
        self
    }

    /// Attach an interactive or recording prompts provider.
    #[must_use]
    pub fn with_prompts(mut self, prompts: Box<dyn PromptsProvider>) -> Self {
        self.prompts = Some(prompts);
        self
    }

    /// Append a log line and emit tracing at a level matching the text.
    ///
    /// Lines that look like errors or warnings use `error!` / `warn!`; everything else is `info!`.
    pub fn log(&mut self, msg: impl AsRef<str>) {
        let s = msg.as_ref().to_string();
        emit_host_log(&s);
        self.logs.push(s);
    }

    /// Record a planned action, log it with a `[plan]` prefix, and push onto [`Self::planned`].
    pub fn plan_action(&mut self, summary: impl Into<String>) {
        let action = PlannedAction::new(summary);
        self.log(format!("[plan] {}", action.summary));
        self.planned.push(action);
    }

    /// True when [`Self::mode`] is [`ApplyMode::DryRun`].
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.mode == ApplyMode::DryRun
    }

    /// Resolve a prompt: prefer [`Self::prompt_answers`], then the prompts provider, else `default`.
    pub fn prompt(&mut self, label: &str, default: &str) -> String {
        if let Some(ans) = self.prompt_answers.get(label) {
            return ans.clone();
        }
        if let Some(ref mut p) = self.prompts {
            return p.prompt(label, default);
        }
        default.to_string()
    }

    /// Resolve a yes/no confirm.
    ///
    /// When [`Self::apply_nat`] is set and `question` contains `"nat"` (ASCII case-insensitive),
    /// returns `true` without asking. Otherwise uses the prompts provider or `default_yes`.
    pub fn confirm(&mut self, question: &str, default_yes: bool) -> bool {
        if self.apply_nat && question.to_ascii_lowercase().contains("nat") {
            return true;
        }
        if let Some(ref mut p) = self.prompts {
            return p.confirm(question, default_yes);
        }
        default_yes
    }
}

/// In Apply mode, require effective uid 0.
///
/// # Errors
///
/// Returns [`HortoError::RootRequired`](crate::error::HortoError::RootRequired) when
/// `mode` is [`ApplyMode::Apply`] and the process is not root.
pub fn require_root_for_apply(mode: ApplyMode) -> crate::error::Result<()> {
    if mode == ApplyMode::Apply && !is_root() {
        return Err(crate::error::HortoError::RootRequired);
    }
    Ok(())
}

/// True when the process effective uid is 0 (Unix). Non-Unix always returns `false`.
#[must_use]
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn emit_host_log(s: &str) {
    match host_log_level(s) {
        HostLogLevel::Error => tracing::error!("{s}"),
        HostLogLevel::Warn => tracing::warn!("{s}"),
        HostLogLevel::Info => tracing::info!("{s}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostLogLevel {
    Error,
    Warn,
    Info,
}

fn host_log_level(msg: &str) -> HostLogLevel {
    let t = msg.trim_start();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("error:")
        || lower.starts_with("error ")
        || lower.contains("command failed")
        || lower.contains("a password is required")
        || lower.contains("permission denied")
    {
        HostLogLevel::Error
    } else if lower.starts_with("warning:")
        || lower.starts_with("warning ")
        || lower.starts_with("warn:")
    {
        HostLogLevel::Warn
    } else {
        HostLogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;

    struct RecordingPrompts {
        prompt_calls: Vec<(String, String)>,
        confirm_calls: Vec<(String, bool)>,
    }

    impl RecordingPrompts {
        fn new() -> Self {
            Self {
                prompt_calls: Vec::new(),
                confirm_calls: Vec::new(),
            }
        }
    }

    impl PromptsProvider for RecordingPrompts {
        fn prompt(&mut self, label: &str, default: &str) -> String {
            self.prompt_calls.push((label.into(), default.into()));
            format!("answer::{label}")
        }

        fn confirm(&mut self, question: &str, default_yes: bool) -> bool {
            self.confirm_calls.push((question.into(), default_yes));
            !default_yes
        }
    }

    #[test]
    fn planned_action_summary_roundtrip() {
        let action = PlannedAction::new("run apt update");
        assert_eq!(action.summary, "run apt update");
    }

    #[test]
    fn host_log_level_classifies_error_and_warning() {
        assert_eq!(host_log_level("ERROR: missing file"), HostLogLevel::Error);
        assert_eq!(
            host_log_level("sudo: a password is required"),
            HostLogLevel::Error
        );
        assert_eq!(
            host_log_level("warning: netplan apply returned non-zero"),
            HostLogLevel::Warn
        );
        assert_eq!(host_log_level("Step d2 complete"), HostLogLevel::Info);
    }

    #[test]
    fn ctx_log_records_body_regardless_of_level() {
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal);
        ctx.log("ERROR: boom");
        ctx.log("warning: soft");
        ctx.log("ok");
        assert_eq!(ctx.logs, vec!["ERROR: boom", "warning: soft", "ok"]);
    }

    #[test]
    fn context_new_defaults() {
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal);
        assert!(ctx.is_dry_run());
        assert!(!ctx.skip_piper);
        assert!(!ctx.stack_opts.any());
        assert!(ctx.ecosystem.status_api && ctx.ecosystem.mcp);
        assert_eq!(ctx.logs, Vec::<String>::new());
        assert_eq!(ctx.planned.len(), 0);
    }

    #[test]
    fn context_log_records_and_stores() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        ctx.log("hello");
        assert_eq!(ctx.logs, vec!["hello".to_string()]);
    }

    #[test]
    fn context_plan_action_logs_dry_run_prefix() {
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full);
        ctx.plan_action("something");
        assert_eq!(ctx.planned.len(), 1);
        assert!(ctx.logs.iter().any(|l| l.contains("[plan] something")));
    }

    #[test]
    fn context_with_paths_replaces_host_paths() {
        let paths = HostPaths::default();
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths.clone());
        assert_eq!(ctx.paths.active_setup, paths.active_setup);
    }

    #[test]
    fn context_prompt_prefers_prefilled_answers_over_prompts_provider() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_prompts(Box::new(RecordingPrompts::new()));
        ctx.prompt_answers
            .insert("label".into(), "prefilled".into());
        assert_eq!(ctx.prompt("label", "default"), "prefilled");
    }

    #[test]
    fn context_prompt_delegates_to_provider() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_prompts(Box::new(RecordingPrompts::new()));
        assert_eq!(ctx.prompt("hostname", "def"), "answer::hostname");
    }

    #[test]
    fn context_prompt_without_provider_returns_default() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        assert_eq!(ctx.prompt("x", "fallback"), "fallback");
    }

    #[test]
    fn context_confirm_apply_nat_short_circuits_when_question_matches() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        ctx.apply_nat = true;
        assert!(ctx.confirm("Apply NAT rules now?", false));
    }

    #[test]
    fn context_confirm_apply_nat_ignores_unrelated_questions() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        ctx.apply_nat = true;
        assert!(!ctx.confirm("Restart dnsmasq?", false));
        assert!(ctx.confirm("Restart dnsmasq?", true));
    }

    #[test]
    fn context_confirm_delegates_to_provider() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full)
            .with_prompts(Box::new(RecordingPrompts::new()));
        // RecordingPrompts.confirm flips default_yes.
        assert!(!ctx.confirm("Anything?", true));
        assert!(ctx.confirm("Anything?", false));
    }

    #[test]
    fn context_confirm_without_provider_returns_default() {
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full);
        assert!(ctx.confirm("q", true));
        assert!(!ctx.confirm("q", false));
    }

    #[test]
    fn non_interactive_prompts_return_defaults() {
        let mut prompts = NonInteractivePrompts;
        assert_eq!(prompts.prompt("label", "default"), "default");
        assert!(prompts.confirm("q?", true));
        assert!(!prompts.confirm("q?", false));
    }

    #[test]
    fn require_root_for_apply_dry_run_is_ok() {
        require_root_for_apply(ApplyMode::DryRun).unwrap();
    }

    #[test]
    fn require_root_for_apply_apply_mode_matches_is_root() {
        let result = require_root_for_apply(ApplyMode::Apply);
        assert_eq!(result.is_ok(), is_root());
    }

    #[test]
    fn is_root_returns_bool() {
        // Just exercise the code path; value depends on env.
        let _ = is_root();
    }
}
