use crate::paths::HostPaths;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    DryRun,
    Apply,
}

#[derive(Debug, Clone)]
pub struct PlannedAction {
    pub summary: String,
}

impl PlannedAction {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }
}

/// Optional interactive prompts. Non-interactive defaults skip prompts.
pub trait PromptsProvider: Send {
    fn prompt(&mut self, label: &str, default: &str) -> String;
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
        use std::io::{self, Write};
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
        use std::io::{self, Write};
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
            "" => default_yes,
            _ => default_yes,
        }
    }
}

pub struct HostContext {
    pub mode: ApplyMode,
    pub paths: HostPaths,
    pub setup_kind: crate::pipeline::SetupKind,
    pub skip_piper: bool,
    pub apply_nat: bool,
    pub logs: Vec<String>,
    pub planned: Vec<PlannedAction>,
    prompts: Option<Box<dyn PromptsProvider>>,
    /// Pre-filled answers keyed by prompt label (tests / automation).
    pub prompt_answers: HashMap<String, String>,
}

impl HostContext {
    pub fn new(mode: ApplyMode, kind: crate::pipeline::SetupKind) -> Self {
        Self {
            mode,
            paths: HostPaths::default(),
            setup_kind: kind,
            skip_piper: false,
            apply_nat: std::env::var("HORTO_APPLY_NAT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            logs: Vec::new(),
            planned: Vec::new(),
            prompts: None,
            prompt_answers: HashMap::new(),
        }
    }

    pub fn with_paths(mut self, paths: HostPaths) -> Self {
        self.paths = paths;
        self
    }

    pub fn with_prompts(mut self, prompts: Box<dyn PromptsProvider>) -> Self {
        self.prompts = Some(prompts);
        self
    }

    pub fn log(&mut self, msg: impl AsRef<str>) {
        let s = msg.as_ref().to_string();
        tracing::info!("{s}");
        self.logs.push(s);
    }

    pub fn plan_action(&mut self, summary: impl Into<String>) {
        let action = PlannedAction::new(summary);
        self.log(format!("[plan] {}", action.summary));
        self.planned.push(action);
    }

    pub fn is_dry_run(&self) -> bool {
        self.mode == ApplyMode::DryRun
    }

    pub fn prompt(&mut self, label: &str, default: &str) -> String {
        if let Some(ans) = self.prompt_answers.get(label) {
            return ans.clone();
        }
        if let Some(ref mut p) = self.prompts {
            return p.prompt(label, default);
        }
        default.to_string()
    }

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

/// In Apply mode, require uid 0.
pub fn require_root_for_apply(mode: ApplyMode) -> crate::error::Result<()> {
    if mode == ApplyMode::Apply && !is_root() {
        return Err(crate::error::HortoError::RootRequired);
    }
    Ok(())
}

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
    fn context_new_defaults() {
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Minimal);
        assert!(ctx.is_dry_run());
        assert!(!ctx.skip_piper);
        assert!(ctx.logs.is_empty());
        assert!(ctx.planned.is_empty());
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
