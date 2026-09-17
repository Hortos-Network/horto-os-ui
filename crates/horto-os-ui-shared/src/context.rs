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
        eprintln!("{s}");
        self.logs.push(s);
    }

    pub fn plan_action(&mut self, summary: impl Into<String>) {
        let action = PlannedAction::new(summary);
        self.log(format!("[dry-run] {}", action.summary));
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
