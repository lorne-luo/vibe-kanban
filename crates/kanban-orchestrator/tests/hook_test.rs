mod common;

use std::{path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use kanban_orchestrator::{
    config::Workflow,
    dispatcher::{PhaseExecutor, TurnOutcome, gate::Gate},
    jira::JiraClient,
    notifier::Notifier,
    scheduler::tick::{OrchestratorContext, dispatch_round},
};
use sqlx::Row;
use uuid::Uuid;

struct AlwaysComplete;

#[async_trait]
impl PhaseExecutor for AlwaysComplete {
    async fn run_turn(
        &self,
        _: &Path,
        _: &Path,
        _: Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        Ok(TurnOutcome {
            stdout: "<<KANBAN_PHASE_COMPLETE>>".into(),
            stderr: String::new(),
            session_id: Uuid::new_v4(),
        })
    }
}

fn make_workflow_with_hooks(
    pre: Option<&str>,
    post: Option<&str>,
    on_err: Option<&str>,
) -> Workflow {
    let pre_line = pre
        .map(|s| format!("  pre_dispatch: {s:?}"))
        .unwrap_or_default();
    let post_line = post
        .map(|s| format!("  post_complete: {s:?}"))
        .unwrap_or_default();
    let err_line = on_err
        .map(|s| format!("  on_error: {s:?}"))
        .unwrap_or_default();
    let yml = format!(
        r#"
version: 1
project: HOOK
sync:
  source: jira
  jira:
    site: http://example.com
    project_key: HOOK
    jql: "project = HOOK"
    auth_env:
      email: HOOK_EMAIL
      token: HOOK_TOKEN
hooks:
{pre_line}
{post_line}
{err_line}
columns:
  - name: Coding
    initial: true
    agent: coder
    on_complete: auto
    next: Done
  - name: Done
    terminal: true
"#
    );
    serde_yaml::from_str(&yml).expect("parse workflow")
}

async fn insert_idle_task(pool: &sqlx::SqlitePool, project_id: Uuid, phase: &str) -> Uuid {
    let id = Uuid::new_v4();
    let id_bytes = id.as_bytes().to_vec();
    let proj_bytes = project_id.as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, kanban_phase, phase_state, \
         current_turn, created_at, updated_at) \
         VALUES (?, ?, 'hook-test', 'todo', ?, 'idle', 0, datetime('now'), datetime('now'))",
    )
    .bind(id_bytes)
    .bind(proj_bytes)
    .bind(phase)
    .execute(pool)
    .await
    .expect("insert task");
    id
}

async fn phase_state(pool: &sqlx::SqlitePool, task_id: Uuid) -> String {
    let row = sqlx::query("SELECT phase_state FROM tasks WHERE id = ?")
        .bind(task_id.as_bytes().to_vec())
        .fetch_one(pool)
        .await
        .unwrap();
    row.try_get("phase_state").unwrap()
}

#[tokio::test]
async fn pre_dispatch_hook_failure_aborts_and_errors_task() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "hook-pre-fail").await;
    let task_id = insert_idle_task(&pool, project_id, "Coding").await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    std::fs::write(
        dir.path().join(".agents/agent/coder.md"),
        "---\nname: coder\n---\nDo work",
    )
    .unwrap();

    // pre_dispatch shell command that exits non-zero
    let workflow = make_workflow_with_hooks(Some("exit 1"), None, None);
    let ctx = OrchestratorContext {
        pool: pool.clone(),
        workflow,
        repo_root: dir.path().to_path_buf(),
        jira: Arc::new(JiraClient::new("http://x".into(), "e".into(), "t".into())),
        gate: Arc::new(Gate::new(5, 2)),
        executor: Arc::new(AlwaysComplete),
        notifier: Arc::new(Notifier::new()),
        project_id,
    };

    dispatch_round(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        phase_state(&pool, task_id).await,
        "error",
        "pre_dispatch failure should set phase_state=error"
    );
}

#[tokio::test]
async fn pre_dispatch_hook_success_allows_phase_to_run() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "hook-pre-ok").await;
    let task_id = insert_idle_task(&pool, project_id, "Coding").await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    std::fs::write(
        dir.path().join(".agents/agent/coder.md"),
        "---\nname: coder\n---\nDo work",
    )
    .unwrap();

    // pre_dispatch hook that succeeds
    let workflow = make_workflow_with_hooks(Some("echo pre_dispatch_ok"), None, None);
    let ctx = OrchestratorContext {
        pool: pool.clone(),
        workflow,
        repo_root: dir.path().to_path_buf(),
        jira: Arc::new(JiraClient::new("http://x".into(), "e".into(), "t".into())),
        gate: Arc::new(Gate::new(5, 2)),
        executor: Arc::new(AlwaysComplete),
        notifier: Arc::new(Notifier::new()),
        project_id,
    };

    dispatch_round(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Phase ran and advanced (card moved to Done = terminal, phase_state stays idle)
    let state = phase_state(&pool, task_id).await;
    assert_ne!(state, "running", "task should not be stuck in running");
    assert_ne!(state, "error", "successful hook should not cause error");
}
