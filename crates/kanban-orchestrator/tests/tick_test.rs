mod common;

use std::sync::Arc;

use async_trait::async_trait;
use kanban_orchestrator::{
    dispatcher::{PhaseExecutor, TurnOutcome},
    dispatcher::gate::Gate,
    jira::JiraClient,
    notifier::Notifier,
    scheduler::tick::{OrchestratorContext, dispatch_round},
};
use uuid::Uuid;

struct AlwaysComplete;

#[async_trait]
impl PhaseExecutor for AlwaysComplete {
    async fn run_turn(
        &self,
        _: &std::path::Path,
        _: &std::path::Path,
        _: std::time::Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        Ok(TurnOutcome {
            stdout: "<<KANBAN_PHASE_COMPLETE>>".into(),
            stderr: String::new(),
            session_id: Uuid::new_v4(),
        })
    }
}

#[tokio::test]
async fn idle_card_with_agent_gets_dispatched() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "AP").await;
    let workflow = common::load_test_workflow();

    // Create a task in "Analyzing" column (has agent: analyzer) with phase_state=idle
    let task_id = Uuid::new_v4();
    let id_bytes = task_id.as_bytes().to_vec();
    let project_id_bytes = project_id.as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, kanban_phase, phase_state, \
         current_turn, created_at, updated_at) \
         VALUES (?, ?, 'Test', 'todo', 'Analyzing', 'idle', 0, \
         datetime('now'), datetime('now'))",
    )
    .bind(id_bytes)
    .bind(project_id_bytes)
    .execute(&pool)
    .await
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    // Create agent file so the phase context write succeeds
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    std::fs::write(
        dir.path().join(".agents/agent/analyzer.md"),
        "---\nname: analyzer\n---\nDo analysis",
    )
    .unwrap();

    let ctx = OrchestratorContext {
        pool: pool.clone(),
        workflow,
        repo_root: dir.path().to_path_buf(),
        jira: Arc::new(JiraClient::new(
            "http://localhost".into(),
            "e".into(),
            "t".into(),
        )),
        gate: Arc::new(Gate::new(5, 2)),
        executor: Arc::new(AlwaysComplete),
        notifier: Arc::new(Notifier::new()),
        project_id,
    };

    dispatch_round(&ctx).await.unwrap();

    // Wait briefly for spawned task to complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Check phase advanced — task should NOT still be idle in Analyzing
    use sqlx::Row;
    let row = sqlx::query("SELECT phase_state, kanban_phase FROM tasks WHERE id = ?")
        .bind(task_id.as_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
    let phase_state: String = row.try_get("phase_state").unwrap();
    let kanban_phase: Option<String> = row.try_get("kanban_phase").ok().flatten();

    // The task must have left the initial idle/Analyzing state
    let still_idle_in_analyzing = phase_state == "idle"
        && kanban_phase.as_deref() == Some("Analyzing");
    assert!(
        !still_idle_in_analyzing,
        "task should have been dispatched from Analyzing/idle; \
         got phase_state={phase_state:?} kanban_phase={kanban_phase:?}"
    );
}

#[tokio::test]
async fn idle_card_without_agent_skipped() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "AP").await;
    let workflow = common::load_test_workflow();

    // Create a task in "Todo" column — no agent, should be skipped
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, kanban_phase, phase_state, \
         current_turn, created_at, updated_at) \
         VALUES (?, ?, 'Skipped Task', 'todo', 'Todo', 'idle', 0, \
         datetime('now'), datetime('now'))",
    )
    .bind(task_id.as_bytes().to_vec())
    .bind(project_id.as_bytes().to_vec())
    .execute(&pool)
    .await
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let ctx = OrchestratorContext {
        pool: pool.clone(),
        workflow,
        repo_root: dir.path().to_path_buf(),
        jira: Arc::new(JiraClient::new(
            "http://localhost".into(),
            "e".into(),
            "t".into(),
        )),
        gate: Arc::new(Gate::new(5, 2)),
        executor: Arc::new(AlwaysComplete),
        notifier: Arc::new(Notifier::new()),
        project_id,
    };

    dispatch_round(&ctx).await.unwrap();

    // Task should remain idle (no agent to dispatch)
    use sqlx::Row;
    let row = sqlx::query("SELECT phase_state FROM tasks WHERE id = ?")
        .bind(task_id.as_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
    let phase_state: String = row.try_get("phase_state").unwrap();
    assert_eq!(phase_state, "idle", "agentless column should remain idle");
}
