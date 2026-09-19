//! Injectable process runner for OpenSSH / curl / tar (unit-testable).

use crate::error::{HortoError, Result};
use std::process::{Command, Stdio};

/// How child stdio is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdioMode {
    /// Pipe stdout/stderr so the parent can read them (probes, downloads).
    Capture,
    /// Inherit stdin/stdout/stderr so OpenSSH can prompt on a TTY.
    Inherit,
}

/// Captured stdout/stderr and exit status from an external program.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Process exit code (or -1 when the OS status had no code).
    pub status: i32,
    /// UTF-8 lossy stdout (empty when [`StdioMode::Inherit`]).
    pub stdout: String,
    /// UTF-8 lossy stderr (empty when [`StdioMode::Inherit`]).
    pub stderr: String,
}

impl CommandOutput {
    /// Whether the process exited with status 0.
    #[must_use]
    pub const fn success(&self) -> bool {
        self.status == 0
    }
}

/// Runs external programs (real OS or a test double).
pub trait ProcessRunner {
    /// Execute `program` with `args`, optional env, and stdio mode.
    ///
    /// # Errors
    ///
    /// Returns [`HortoError`] when the process cannot be spawned.
    fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        stdio: StdioMode,
    ) -> Result<CommandOutput>;
}

/// Default runner that spawns real OS processes.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        stdio: StdioMode,
    ) -> Result<CommandOutput> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        match stdio {
            StdioMode::Capture => {
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let out = cmd
                    .output()
                    .map_err(|e| HortoError::command(program, e.to_string()))?;
                Ok(CommandOutput {
                    status: out.status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                })
            }
            StdioMode::Inherit => {
                cmd.stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                let status = cmd
                    .status()
                    .map_err(|e| HortoError::command(program, e.to_string()))?;
                Ok(CommandOutput {
                    status: status.code().unwrap_or(-1),
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }
    }
}

/// One recorded process invocation (tests).
#[cfg(test)]
pub type RecordedCall = (String, Vec<String>, Vec<(String, String)>, StdioMode);

/// Scripted runner with a pop-able reply queue per program (tests).
#[cfg(test)]
#[derive(Debug, Default)]
pub struct ScriptedRunner {
    scripts: std::sync::Mutex<std::collections::HashMap<String, Vec<CommandOutput>>>,
    /// Recorded process invocations.
    pub calls: std::sync::Mutex<Vec<RecordedCall>>,
}

#[cfg(test)]
impl ScriptedRunner {
    pub fn push(&self, program: &str, output: CommandOutput) {
        self.scripts
            .lock()
            .expect("lock")
            .entry(program.to_owned())
            .or_default()
            .push(output);
    }

    pub fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    pub fn fail(status: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }
}

#[cfg(test)]
impl ProcessRunner for ScriptedRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        stdio: StdioMode,
    ) -> Result<CommandOutput> {
        self.calls.lock().expect("lock").push((
            program.to_owned(),
            args.iter().map(|s| (*s).to_owned()).collect(),
            env.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            stdio,
        ));
        let mut map = self.scripts.lock().expect("lock");
        let queue = map.get_mut(program).ok_or_else(|| {
            HortoError::msg(format!("scripted runner: unexpected program {program}"))
        })?;
        if queue.is_empty() {
            return Err(HortoError::msg(format!(
                "scripted runner: no more replies for {program}"
            )));
        }
        Ok(queue.remove(0))
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    #[test]
    fn command_output_success() {
        assert!(CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
        .success());
        assert!(!CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: String::new(),
        }
        .success());
    }

    #[test]
    fn system_runner_capture_echo() {
        let out = SystemProcessRunner
            .run("echo", &["hello-remote"], &[], StdioMode::Capture)
            .unwrap();
        assert!(out.success());
        assert!(out.stdout.contains("hello-remote"));
    }

    #[test]
    fn system_runner_inherit_true() {
        let out = SystemProcessRunner
            .run("true", &[], &[], StdioMode::Inherit)
            .unwrap();
        assert!(out.success());
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn system_runner_missing_program() {
        let err = SystemProcessRunner
            .run(
                "horto-os-ui-definitely-missing-bin-xyz",
                &[],
                &[],
                StdioMode::Capture,
            )
            .unwrap_err();
        assert!(err.to_string().contains("horto-os-ui-definitely-missing"));
    }

    #[test]
    fn scripted_unexpected_and_exhausted() {
        let runner = ScriptedRunner::default();
        let err = runner
            .run("nope", &[], &[], StdioMode::Capture)
            .unwrap_err();
        assert!(err.to_string().contains("unexpected"));
        runner.push("once", ScriptedRunner::ok(""));
        runner.run("once", &[], &[], StdioMode::Capture).unwrap();
        let err = runner
            .run("once", &[], &[], StdioMode::Capture)
            .unwrap_err();
        assert!(err.to_string().contains("no more replies"));
    }
}
