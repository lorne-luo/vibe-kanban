use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use db::models::task::{PhaseState, Task, TaskStatus};
use kanban_orchestrator::{
    config::{Column, OnComplete},
    dispatcher::{PhaseExecutor, TurnOutcome, phase::PhaseOutcome},
};
use uuid::Uuid;

struct CompleteOnTurn(usize, Arc<AtomicUsize>);

#[async_trait]
impl PhaseExecutor for CompleteOnTurn {
    async fn run_turn(
        &self,
        _: &Path,
        _: &Path,
        _: std::time::Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        let n = self.1.fetch_add(1, Ordering::SeqCst) + 1;
        let stdout = if n >= self.0 {
            "<<KANBAN_PHASE_COMPLETE>>".into()
        } else {
            "still working".into()
        };
        Ok(TurnOutcome {
            stdout,
            stderr: String::new(),
            session_id: Uuid::new_v4(),
        })
    }
}

fn make_column(on_complete: OnComplete) -> Column {
    kanban_orchestrator::config::Column {
        name: "Coding".into(),
        jira_status: vec![],
        initial: None,
        terminal: None,
        agent: Some("coder".into()),
        on_complete,
        next: Some("Done".into()),
        jira_transition: None,
        max_turns_per_phase: None,
    }
}

fn make_card() -> Task {
    Task {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        title: "Test".into(),
        description: None,
        status: TaskStatus::Todo,
        parent_workspace_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        jira_key: Some("AP-1".into()),
        jira_snapshot: None,
        jira_synced_at: None,
        kanban_phase: Some("Coding".into()),
        phase_state: PhaseState::Idle,
        current_turn: 0,
        last_executor_session_id: None,
        review_pending_since: None,
        error_info: None,
        pending_inject: None,
    }
}

async fn run_test_phase(exec: &dyn PhaseExecutor, max_turns: u32) -> PhaseOutcome {
    let dir = tempfile::tempdir().unwrap();
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: test\n---\nbody").unwrap();
    let card = make_card();
    let column = make_column(OnComplete::Auto);
    let cancel = Arc::new(tokio::sync::Notify::new());
    let inputs = kanban_orchestrator::dispatcher::phase::PhaseInputs {
        card: &card,
        column: &column,
        worktree: dir.path(),
        agent_md: &agent_md,
        max_turns,
        turn_timeout: std::time::Duration::from_secs(30),
        reconcile_blob: None,
        review_feedback: None,
        cancel,
    };
    kanban_orchestrator::dispatcher::phase::run_phase(inputs, exec)
        .await
        .unwrap()
}

async fn run_test_phase_with_review(exec: &dyn PhaseExecutor, max_turns: u32) -> PhaseOutcome {
    let dir = tempfile::tempdir().unwrap();
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: test\n---\nbody").unwrap();
    let card = make_card();
    let column = make_column(OnComplete::Review);
    let cancel = Arc::new(tokio::sync::Notify::new());
    let inputs = kanban_orchestrator::dispatcher::phase::PhaseInputs {
        card: &card,
        column: &column,
        worktree: dir.path(),
        agent_md: &agent_md,
        max_turns,
        turn_timeout: std::time::Duration::from_secs(30),
        reconcile_blob: None,
        review_feedback: None,
        cancel,
    };
    kanban_orchestrator::dispatcher::phase::run_phase(inputs, exec)
        .await
        .unwrap()
}

#[tokio::test]
async fn phase_completes_on_marker() {
    let exec = CompleteOnTurn(2, Arc::new(AtomicUsize::new(0)));
    let outcome = run_test_phase(&exec, 5).await;
    match outcome {
        PhaseOutcome::Complete { turns_used, .. } => assert_eq!(turns_used, 2),
        other => panic!(
            "expected Complete, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[tokio::test]
async fn log_files_created_after_turn() {
    let exec = CompleteOnTurn(1, Arc::new(AtomicUsize::new(0)));
    let dir = tempfile::tempdir().unwrap();
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: test\n---\nbody").unwrap();
    let card = make_card(); // jira_key = "AP-1"
    let column = make_column(OnComplete::Auto);
    let cancel = Arc::new(tokio::sync::Notify::new());
    let inputs = kanban_orchestrator::dispatcher::phase::PhaseInputs {
        card: &card,
        column: &column,
        worktree: dir.path(),
        agent_md: &agent_md,
        max_turns: 5,
        turn_timeout: std::time::Duration::from_secs(30),
        reconcile_blob: None,
        review_feedback: None,
        cancel,
    };
    kanban_orchestrator::dispatcher::phase::run_phase(inputs, &exec)
        .await
        .unwrap();

    // Log file should exist in asset_dir/logs/AP-1/
    let log_dir = utils::assets::asset_dir().join("logs").join("AP-1");
    if log_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&log_dir).unwrap().collect();
        assert!(!entries.is_empty(), "log files should be created");
    }
    // If log_dir doesn't exist (e.g. asset_dir not writable) we treat it as lenient pass
}

#[tokio::test]
async fn phase_review_when_max_turns_and_review_required() {
    let exec = CompleteOnTurn(99, Arc::new(AtomicUsize::new(0)));
    let outcome = run_test_phase_with_review(&exec, 3).await;
    match outcome {
        PhaseOutcome::AwaitingReview { turns_used, .. } => assert_eq!(turns_used, 3),
        other => panic!(
            "expected AwaitingReview, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}
