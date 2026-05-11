//! Adapter that wraps a [`CodingAgent`] executor as a [`PhaseExecutor`].
//!
//! The adapter reads the agent markdown file, extracts the prompt body (text
//! after the YAML front matter), spawns one executor turn, collects stdout,
//! and returns a [`TurnOutcome`].
//!
//! Session-ID extraction: claude-code embeds the session UUID in a JSON log
//! line.  We scan stdout for the first occurrence of `"session_id":"<uuid>"`
//! and parse it.  If not found we generate a fresh UUID (covers non-claude
//! executors and unit tests).

use std::{path::Path, time::Duration};

use async_trait::async_trait;
use executors::{
    env::{ExecutionEnv, RepoContext},
    executors::{CodingAgent, StandardCodingAgentExecutor},
};
use futures::StreamExt;
use tokio::io::AsyncBufReadExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::dispatcher::{PhaseExecutor, TurnOutcome};

/// A real executor adapter that drives one agent turn per [`run_turn`] call.
pub struct RealExecutor {
    /// The underlying coding-agent variant (ClaudeCode, Gemini, etc.)
    pub agent: CodingAgent,
    /// Session ID from the *previous* turn, used for `spawn_follow_up`.
    /// `None` for the very first turn.
    pub session_id: Option<String>,
    /// Execution environment (env vars, repo context).
    pub env: ExecutionEnv,
}

impl RealExecutor {
    /// Construct a new adapter for the first turn of a phase.
    pub fn new(agent: CodingAgent, env: ExecutionEnv) -> Self {
        Self {
            agent,
            session_id: None,
            env,
        }
    }

    /// Construct with a worktree path and default env (for simple callers).
    pub fn with_worktree(agent: CodingAgent, worktree: &Path) -> Self {
        let repo_context = RepoContext::new(worktree.to_path_buf(), vec![]);
        let env = ExecutionEnv::new(repo_context, false, String::new());
        Self::new(agent, env)
    }
}

/// Extract the prompt body from an agent markdown file.
///
/// Format expected:
/// ```text
/// ---
/// name: coder
/// ---
/// <prompt body starts here>
/// ```
/// If no front matter is found, the entire file content is returned as-is.
pub fn prompt_from_agent_md(content: &str) -> &str {
    // Look for the closing `---` of the front matter block
    if let Some(rest) = content.strip_prefix("---") {
        // Find the closing delimiter
        if let Some(idx) = rest.find("\n---") {
            let body_start = idx + "\n---".len();
            let body = &rest[body_start..];
            // Skip a single leading newline if present
            return body.strip_prefix('\n').unwrap_or(body);
        }
    }
    content
}

/// Try to find a Claude-style session UUID from a JSON log line.
///
/// Claude emits lines like: `{"session_id":"<uuid>", ...}`
fn extract_session_id(stdout: &str) -> Option<Uuid> {
    // Fast scan: find `"session_id":"` then parse the following token
    let needle = r#""session_id":""#;
    let idx = stdout.find(needle)?;
    let tail = &stdout[idx + needle.len()..];
    let end = tail.find('"')?;
    Uuid::parse_str(&tail[..end]).ok()
}

#[async_trait]
impl PhaseExecutor for RealExecutor {
    async fn run_turn(
        &self,
        worktree: &Path,
        agent_md_path: &Path,
        timeout: Duration,
    ) -> crate::Result<TurnOutcome> {
        // 1. Read the agent prompt body
        let md_content = std::fs::read_to_string(agent_md_path)?;
        let prompt = prompt_from_agent_md(&md_content).to_string();

        // 2. Spawn the executor turn
        let mut spawned = match &self.session_id {
            None => self
                .agent
                .spawn(worktree, &prompt, &self.env)
                .await
                .map_err(|e| crate::OrchestratorError::Executor(e.to_string()))?,
            Some(sid) => self
                .agent
                .spawn_follow_up(worktree, &prompt, sid, None, &self.env)
                .await
                .map_err(|e| crate::OrchestratorError::Executor(e.to_string()))?,
        };

        // 3. Collect stdout with a timeout
        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();

        // Take stdout from the child; stderr collected separately where available
        if let Some(raw_stdout) = spawned.child.inner().stdout.take() {
            let mut reader = tokio::io::BufReader::new(raw_stdout);
            let mut line = String::new();

            let result = tokio::time::timeout(timeout, async {
                loop {
                    line.clear();
                    let n = reader.read_line(&mut line).await?;
                    if n == 0 {
                        break; // EOF
                    }
                    stdout_buf.push_str(&line);
                }
                Ok::<(), std::io::Error>(())
            })
            .await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(io_err)) => {
                    return Err(crate::OrchestratorError::Io(io_err));
                }
                Err(_elapsed) => {
                    // Kill child on timeout
                    let _ = spawned.child.kill();
                    return Err(crate::OrchestratorError::ExecutorTimeout(timeout));
                }
            }
        }

        // 4. Collect stderr (best-effort; ignore errors)
        if let Some(raw_stderr) = spawned.child.inner().stderr.take() {
            let mut stream = ReaderStream::new(raw_stderr);
            while let Some(Ok(chunk)) = stream.next().await {
                if let Ok(s) = std::str::from_utf8(&chunk) {
                    stderr_buf.push_str(s);
                }
            }
        }

        // 5. Wait for child to finish
        let _ = spawned.child.wait().await;

        // 6. Determine session UUID
        let session_id = extract_session_id(&stdout_buf).unwrap_or_else(Uuid::new_v4);

        Ok(TurnOutcome {
            stdout: stdout_buf,
            stderr: stderr_buf,
            session_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_from_agent_md_strips_front_matter() {
        let md = "---\nname: coder\n---\nDo the work!";
        assert_eq!(prompt_from_agent_md(md), "Do the work!");
    }

    #[test]
    fn prompt_from_agent_md_no_front_matter() {
        let md = "Do the work directly";
        assert_eq!(prompt_from_agent_md(md), "Do the work directly");
    }

    #[test]
    fn extract_session_id_finds_uuid() {
        let line = r#"{"session_id":"550e8400-e29b-41d4-a716-446655440000","type":"system"}"#;
        let id = extract_session_id(line).unwrap();
        assert_eq!(id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn extract_session_id_missing_returns_none() {
        assert!(extract_session_id("no session here").is_none());
    }
}
