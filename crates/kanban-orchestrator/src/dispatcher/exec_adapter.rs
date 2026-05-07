//! Executor adapter — bridges the `executors` crate into `PhaseExecutor`.
//!
//! Two concrete adapters are provided:
//!
//! * [`AgentExecutorAdapter`] — wraps any [`CodingAgent`] from the `executors`
//!   crate. It spawns the child process, reads its stdout to a `String`, then
//!   returns a [`TurnOutcome`].
//!
//! * [`SimpleShellExecutor`] — runs an arbitrary shell command built from a
//!   command template and the prompt read from `agent_md`.  Useful when you
//!   need to call an executor that is not yet integrated with the `CodingAgent`
//!   enum (e.g. a local wrapper script).

use std::{path::Path, process::Stdio, time::Duration};

use async_trait::async_trait;
use tokio::io::AsyncReadExt as _;
use tracing::{debug, warn};

use crate::dispatcher::{PhaseExecutor, TurnOutcome};

// ─── AgentExecutorAdapter ────────────────────────────────────────────────────

/// Adapter that wraps an [`executors::executors::CodingAgent`].
///
/// On each [`PhaseExecutor::run_turn`] call:
/// 1. Reads the `agent_md` file to obtain the prompt.
/// 2. Calls [`StandardCodingAgentExecutor::spawn`] on the wrapped agent.
/// 3. Reads all stdout bytes while the process runs.
/// 4. Waits for the child to exit.
/// 5. Returns [`TurnOutcome`] with the captured stdout/stderr and a fresh UUID
///    as `session_id` (the executors crate embeds the real session ID *inside*
///    the JSON log stream, which a higher layer can parse if needed).
pub struct AgentExecutorAdapter {
    agent: executors::executors::CodingAgent,
    env: executors::env::ExecutionEnv,
}

impl AgentExecutorAdapter {
    /// Create a new adapter for the given coding agent and execution
    /// environment.
    pub fn new(
        agent: executors::executors::CodingAgent,
        env: executors::env::ExecutionEnv,
    ) -> Self {
        Self { agent, env }
    }
}

#[async_trait]
impl PhaseExecutor for AgentExecutorAdapter {
    async fn run_turn(
        &self,
        worktree: &Path,
        agent_md_path: &Path,
        timeout: Duration,
    ) -> crate::Result<TurnOutcome> {
        use executors::executors::StandardCodingAgentExecutor as _;

        let prompt = tokio::fs::read_to_string(agent_md_path)
            .await
            .map_err(crate::OrchestratorError::Io)?;

        debug!(
            worktree = %worktree.display(),
            agent_md = %agent_md_path.display(),
            "AgentExecutorAdapter: spawning agent"
        );

        let mut spawned = self
            .agent
            .spawn(worktree, &prompt, &self.env)
            .await
            .map_err(|e| {
                crate::OrchestratorError::Other(anyhow::anyhow!("executor spawn error: {e}"))
            })?;

        // Take ownership of stdout/stderr handles before waiting.
        let stdout_handle = spawned.child.inner().stdout.take();
        let stderr_handle = spawned.child.inner().stderr.take();

        // Read stdout and stderr concurrently, with a timeout guard.
        let read_fut = async {
            let stdout = match stdout_handle {
                Some(mut h) => {
                    let mut buf = String::new();
                    let _ = h.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            };
            let stderr = match stderr_handle {
                Some(mut h) => {
                    let mut buf = String::new();
                    let _ = h.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            };
            (stdout, stderr)
        };

        let (stdout, stderr) = match tokio::time::timeout(timeout, read_fut).await {
            Ok(pair) => pair,
            Err(_elapsed) => {
                warn!("AgentExecutorAdapter: timeout waiting for executor stdout");
                // Best-effort kill and return what we have (empty in this path).
                let _ = spawned.child.kill().await;
                (String::new(), String::new())
            }
        };

        // Wait for the child to fully exit (it should already be done after EOF
        // on stdout, but we still need to reap it).
        let _ = spawned.child.wait().await;

        Ok(TurnOutcome {
            stdout,
            stderr,
            session_id: uuid::Uuid::new_v4(),
        })
    }
}

// ─── SimpleShellExecutor ─────────────────────────────────────────────────────

/// A simple shell-based executor.
///
/// Runs the given `command` (split by whitespace) with the prompt appended as
/// a final argument, captures stdout/stderr, and returns the result.
///
/// This is useful for ad-hoc integration tests or for invoking any coding
/// agent CLI that is not yet wrapped in `CodingAgent`.
///
/// # Example
///
/// ```rust,no_run
/// use std::path::Path;
/// use kanban_orchestrator::dispatcher::exec_adapter::SimpleShellExecutor;
///
/// let exec = SimpleShellExecutor::new("claude --print");
/// ```
pub struct SimpleShellExecutor {
    /// Full command line *without* the prompt.  The prompt will be appended as
    /// a single extra argument when the process is spawned.
    ///
    /// Example: `"claude --output-format stream-json --print"`
    command: String,
}

impl SimpleShellExecutor {
    /// Create a new executor that will invoke `command` followed by the prompt.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

#[async_trait]
impl PhaseExecutor for SimpleShellExecutor {
    async fn run_turn(
        &self,
        worktree: &Path,
        agent_md_path: &Path,
        timeout: Duration,
    ) -> crate::Result<TurnOutcome> {
        let prompt = tokio::fs::read_to_string(agent_md_path)
            .await
            .map_err(crate::OrchestratorError::Io)?;

        // Split the stored command string into program + args.
        let mut parts = self.command.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| crate::OrchestratorError::Workflow("empty command".into()))?;
        let base_args: Vec<&str> = parts.collect();

        debug!(
            worktree = %worktree.display(),
            command = %self.command,
            "SimpleShellExecutor: spawning"
        );

        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&base_args)
            .arg(&prompt)
            .current_dir(worktree)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(crate::OrchestratorError::Io)?;

        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let read_fut = async {
            let stdout = match stdout_handle {
                Some(mut h) => {
                    let mut buf = String::new();
                    let _ = h.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            };
            let stderr = match stderr_handle {
                Some(mut h) => {
                    let mut buf = String::new();
                    let _ = h.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            };
            (stdout, stderr)
        };

        let (stdout, stderr) = match tokio::time::timeout(timeout, read_fut).await {
            Ok(pair) => pair,
            Err(_elapsed) => {
                warn!("SimpleShellExecutor: timeout waiting for process stdout");
                let _ = child.kill().await;
                (String::new(), String::new())
            }
        };

        let _ = child.wait().await;

        Ok(TurnOutcome {
            stdout,
            stderr,
            session_id: uuid::Uuid::new_v4(),
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Helper: write agent_md and return its path + the temp dir guard.
    fn make_agent_md(content: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("agent.md");
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    #[tokio::test]
    async fn simple_shell_executor_captures_echo() {
        // `echo` is available on all supported platforms (macOS, Linux).
        let exec = SimpleShellExecutor::new("echo");
        let (_dir, agent_md) = make_agent_md("hello world");
        // Use a worktree path that exists (the tempdir itself).
        let outcome = exec
            .run_turn(
                _dir.path(),
                &agent_md,
                Duration::from_secs(10),
            )
            .await
            .expect("run_turn should succeed");

        assert!(
            outcome.stdout.contains("hello world"),
            "stdout should contain the echoed prompt, got: {:?}",
            outcome.stdout
        );
    }

    #[tokio::test]
    async fn simple_shell_executor_timeout_kills_process() {
        // `sleep 60` should be killed well before 60 s because we give 1 ms.
        let exec = SimpleShellExecutor::new("sleep");
        let (_dir, agent_md) = make_agent_md("60");
        let outcome = exec
            .run_turn(
                _dir.path(),
                &agent_md,
                Duration::from_millis(200),
            )
            .await
            .expect("run_turn should return Ok even on timeout");

        // After timeout stdout is empty.
        assert!(outcome.stdout.is_empty());
    }
}
