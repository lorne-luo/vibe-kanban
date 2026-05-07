pub mod gate;
pub mod markers;
pub mod phase;

#[async_trait::async_trait]
pub trait PhaseExecutor: Send + Sync + 'static {
    async fn run_turn(
        &self,
        worktree: &std::path::Path,
        agent_md_path: &std::path::Path,
        timeout: std::time::Duration,
    ) -> crate::Result<TurnOutcome>;
}

pub struct TurnOutcome {
    pub stdout: String,
    pub stderr: String,
    pub session_id: uuid::Uuid,
}
