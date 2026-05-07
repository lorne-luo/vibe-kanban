use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use kanban_orchestrator::{
    config::{Column, OnComplete},
    dispatcher::{
        PhaseExecutor, TurnOutcome,
        phase::{PhaseInputs, PhaseOutcome, run_phase},
    },
};
use tokio::sync::Notify;

struct CompleteOnTurn {
    target: usize,
    counter: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl PhaseExecutor for CompleteOnTurn {
    async fn run_turn(
        &self,
        _worktree: &Path,
        _agent_md: &Path,
        _timeout: Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let stdout = if n >= self.target {
            "<<KANBAN_PHASE_COMPLETE>>".into()
        } else {
            "still working".into()
        };
        Ok(TurnOutcome {
            stdout,
            stderr: String::new(),
            session_id: uuid::Uuid::new_v4(),
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
        next: Some("Reviewing".into()),
        jira_transition: None,
        max_turns_per_phase: None,
    }
}

async fn run_test_phase(
    exec: &dyn PhaseExecutor,
    max_turns: u32,
    on_complete: OnComplete,
) -> PhaseOutcome {
    let dir = tempfile::tempdir().unwrap();
    let card = make_task();
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: coder\n---\ndo the thing").unwrap();
    let cancel = Arc::new(Notify::new());
    let column = make_column(on_complete);
    let inputs = PhaseInputs {
        card: &card,
        column: &column,
        worktree: dir.path(),
        agent_md: &agent_md,
        max_turns,
        turn_timeout: Duration::from_secs(30),
        reconcile_blob: None,
        review_feedback: None,
        cancel,
    };
    run_phase(inputs, exec).await.unwrap()
}

fn make_task() -> db::models::task::Task {
    db::models::task::Task {
        id: uuid::Uuid::new_v4(),
        project_id: uuid::Uuid::new_v4(),
        title: "test".into(),
        description: None,
        status: db::models::task::TaskStatus::Todo,
        parent_workspace_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        jira_key: Some("AP-1".into()),
        jira_snapshot: None,
        jira_synced_at: None,
        kanban_phase: Some("Coding".into()),
        phase_state: db::models::task::PhaseState::Running,
        current_turn: 0,
        last_executor_session_id: None,
        review_pending_since: None,
        error_info: None,
        pending_inject: None,
    }
}

#[tokio::test]
async fn phase_completes_on_marker() {
    let exec = CompleteOnTurn {
        target: 2,
        counter: Arc::new(AtomicUsize::new(0)),
    };
    let outcome = run_test_phase(&exec, 5, OnComplete::Auto).await;
    match outcome {
        PhaseOutcome::Complete { turns_used, .. } => assert_eq!(turns_used, 2),
        other => panic!("expected Complete, got {:?}", other),
    }
}

#[tokio::test]
async fn phase_awaiting_review_at_max_turns_with_review() {
    let exec = CompleteOnTurn {
        target: 99, // never completes
        counter: Arc::new(AtomicUsize::new(0)),
    };
    let outcome = run_test_phase(&exec, 3, OnComplete::Review).await;
    match outcome {
        PhaseOutcome::AwaitingReview { turns_used, .. } => assert_eq!(turns_used, 3),
        other => panic!("expected AwaitingReview, got {:?}", other),
    }
}

#[tokio::test]
async fn phase_auto_advances_at_max_turns() {
    let exec = CompleteOnTurn {
        target: 99, // never completes
        counter: Arc::new(AtomicUsize::new(0)),
    };
    let outcome = run_test_phase(&exec, 2, OnComplete::Auto).await;
    match outcome {
        PhaseOutcome::Complete { turns_used, .. } => assert_eq!(turns_used, 2),
        other => panic!("expected Complete (auto), got {:?}", other),
    }
}

#[tokio::test]
async fn phase_writes_turn_log() {
    let exec = CompleteOnTurn {
        target: 1,
        counter: Arc::new(AtomicUsize::new(0)),
    };
    let dir = tempfile::tempdir().unwrap();
    let card = make_task(); // uses jira_key: Some("AP-1")
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: coder\n---\ndo the thing").unwrap();
    let cancel = Arc::new(Notify::new());
    let column = make_column(OnComplete::Auto);
    let inputs = PhaseInputs {
        card: &card,
        column: &column,
        worktree: dir.path(),
        agent_md: &agent_md,
        max_turns: 3,
        turn_timeout: Duration::from_secs(30),
        reconcile_blob: None,
        review_feedback: None,
        cancel,
    };
    run_phase(inputs, &exec).await.unwrap();
    let log = dir.path().join("logs/AP-1/phase-Coding-turn-1.log");
    assert!(log.exists(), "log file should be created");
}
