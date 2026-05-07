mod common;

use std::{path::Path, str::FromStr, sync::Arc, time::Duration};

use kanban_orchestrator::{
    config::Workflow,
    dispatcher::{PhaseExecutor, TurnOutcome, gate::Gate},
    jira::JiraClient,
    notifier::Notifier,
    scheduler::tick::{OrchestratorContext, do_tick},
};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

// ---------------------------------------------------------------------------
// Mock executor — always outputs the complete marker on the first call
// ---------------------------------------------------------------------------

struct AutoCompleteExecutor;

#[async_trait::async_trait]
impl PhaseExecutor for AutoCompleteExecutor {
    async fn run_turn(
        &self,
        _worktree: &Path,
        _agent_md: &Path,
        _timeout: Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        Ok(TurnOutcome {
            stdout: "<<KANBAN_PHASE_COMPLETE>>".into(),
            stderr: String::new(),
            session_id: uuid::Uuid::new_v4(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn jira_issue(key: &str, summary: &str, status: &str) -> serde_json::Value {
    json!({
        "key": key,
        "fields": {
            "summary": summary,
            "status": {"name": status},
            "assignee": null,
            "comment": {"comments": []},
            "description": null,
            "labels": [],
            "priority": null,
            "attachment": []
        }
    })
}

async fn build_ctx(
    pool: sqlx::SqlitePool,
    project_id: uuid::Uuid,
    repo_root: std::path::PathBuf,
    jira_base: String,
    email: &str,
    token: &str,
    workflow: Workflow,
) -> OrchestratorContext {
    OrchestratorContext {
        pool,
        workflow,
        repo_root,
        jira: JiraClient::new(jira_base, email.into(), token.into()),
        gate: Arc::new(Gate::new(3, 1)),
        executor: Arc::new(AutoCompleteExecutor),
        notifier: Arc::new(Notifier::new(false)),
        project_id,
    }
}

fn workflow_yaml() -> String {
    r#"
version: 1
project: AP
sync:
  source: jira
  jira:
    site: placeholder
    project_key: AP
    jql: "project=AP"
    poll_interval: 15m
    auth_env:
      email: JIRA_EMAIL
      token: JIRA_API_TOKEN
columns:
  - name: Todo
    jira_status: ["To Do"]
    initial: true
    next: Analyzing
  - name: Analyzing
    agent: analyzer
    on_complete: auto
    next: Coding
  - name: Coding
    agent: coder
    on_complete: auto
    next: Reviewing
  - name: Reviewing
    agent: reviewer
    on_complete: auto
    next: Done
    jira_transition: "Done"
  - name: Done
    terminal: true
defaults:
  max_concurrent_dispatches: 3
  max_per_column: 1
  max_turns_per_phase: 5
reconciliation:
  on_jira_comment: inject_next_turn
  on_jira_status_terminal: stop_immediately
  on_jira_assignee_change: stop_immediately
  on_jira_summary_change: inject_next_turn
hooks: {}
"#
    .into()
}

async fn setup_wiremock() -> MockServer {
    let server = MockServer::start().await;

    // Jira search always returns AP-42 in "To Do"
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [jira_issue("AP-42", "Build feature X", "To Do")]
        })))
        .mount(&server)
        .await;

    // Jira GET transitions for any issue
    Mock::given(method("GET"))
        .and(path_regex(r"/rest/api/3/issue/.*/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transitions": [
                {"id": "11", "name": "Start Progress"},
                {"id": "31", "name": "Done"}
            ]
        })))
        .mount(&server)
        .await;

    // Jira POST transitions (e.g. advance to Done)
    Mock::given(method("POST"))
        .and(path_regex(r"/rest/api/3/issue/.*/transitions"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    server
}

fn setup_repo_dir(dir: &std::path::Path) {
    // Agent files required by workflow validator
    std::fs::create_dir_all(dir.join(".agents/agent")).unwrap();
    for agent in ["analyzer", "coder", "reviewer"] {
        std::fs::write(
            dir.join(format!(".agents/agent/{}.md", agent)),
            "---\nname: stub\n---\ndo the work",
        )
        .unwrap();
    }

    // Workflow YAML
    std::fs::create_dir_all(dir.join(".agents/kanban-workflows")).unwrap();
    std::fs::write(dir.join(".agents/kanban-workflows/AP.yml"), workflow_yaml()).unwrap();
}

/// Poll the DB until the task reaches `expected` kanban_phase or we time out.
async fn wait_for_phase(pool: &sqlx::SqlitePool, jira_key: &str, expected: &str, attempts: u32) {
    for _ in 0..attempts {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let row: Option<(String,)> =
            sqlx::query_as("SELECT kanban_phase FROM tasks WHERE jira_key = ?")
                .bind(jira_key)
                .fetch_optional(pool)
                .await
                .unwrap();

        if let Some((phase,)) = row {
            if phase == expected {
                return;
            }
        }
    }
    // Let the assertion at the call site provide a clear failure message
}

// ---------------------------------------------------------------------------
// End-to-end test
// ---------------------------------------------------------------------------

/// Full kanban loop: Jira poll → reconcile → phase dispatch → Done.
///
/// Uses a wiremock Jira mock and an `AutoCompleteExecutor` that immediately
/// emits `<<KANBAN_PHASE_COMPLETE>>`.  Verifies that a card progresses
/// Todo → Analyzing → Coding → Reviewing → Done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_card_advances_through_columns() {
    // Set env vars that workflow validator checks.
    // SAFETY: test binary is single-threaded at this point.
    unsafe {
        std::env::set_var("JIRA_EMAIL", "x");
        std::env::set_var("JIRA_API_TOKEN", "y");
    }

    let server = setup_wiremock().await;

    // Use a temp-file SQLite so all pool connections share the same data.
    // In-memory (`:memory:`) databases are per-connection, so we need a real
    // file when running with a multi-connection pool.
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("e2e_test.db");
    let pool = sqlx::pool::PoolOptions::<sqlx::Sqlite>::new()
        .max_connections(5)
        .connect_with(
            SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
                .unwrap()
                .create_if_missing(true)
                .pragma("journal_mode", "WAL"),
        )
        .await
        .unwrap();
    sqlx::migrate!("../db/migrations").run(&pool).await.unwrap();
    let project_id = common::create_project(&pool, "AP").await;

    let dir = tempfile::tempdir().unwrap();
    setup_repo_dir(dir.path());

    let workflow = kanban_orchestrator::config::load_workflow(dir.path(), "AP").unwrap();

    let ctx = build_ctx(
        pool.clone(),
        project_id,
        dir.path().to_path_buf(),
        server.uri(),
        "x",
        "y",
        workflow,
    )
    .await;

    // -----------------------------------------------------------------------
    // Tick 1: Jira poll creates card AP-42 in "Todo" (no agent on Todo)
    // -----------------------------------------------------------------------
    do_tick(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (phase1, _): (String, String) =
        sqlx::query_as("SELECT kanban_phase, phase_state FROM tasks WHERE jira_key = 'AP-42'")
            .fetch_one(&pool)
            .await
            .expect("task AP-42 should exist after first tick");

    assert_eq!(phase1, "Todo", "after tick 1, card should be in Todo");

    // -----------------------------------------------------------------------
    // Tick 2: manually advance card to Analyzing and dispatch
    // AutoCompleteExecutor fires immediately → card moves to Coding
    // -----------------------------------------------------------------------
    sqlx::query(
        "UPDATE tasks SET kanban_phase = 'Analyzing', phase_state = 'idle', current_turn = 0 \
         WHERE jira_key = 'AP-42'",
    )
    .execute(&pool)
    .await
    .unwrap();

    do_tick(&ctx).await.unwrap();
    // Phase dispatch is async (tokio::spawn); wait for it to complete
    wait_for_phase(&pool, "AP-42", "Coding", 30).await;

    let (phase2, state2): (String, String) =
        sqlx::query_as("SELECT kanban_phase, phase_state FROM tasks WHERE jira_key = 'AP-42'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        phase2, "Coding",
        "after Analyzing (auto-complete), card should advance to Coding (state={})",
        state2
    );

    // -----------------------------------------------------------------------
    // Tick 3: Coding phase runs → card moves to Reviewing
    // -----------------------------------------------------------------------
    do_tick(&ctx).await.unwrap();
    wait_for_phase(&pool, "AP-42", "Reviewing", 30).await;

    let (phase3, _): (String, String) =
        sqlx::query_as("SELECT kanban_phase, phase_state FROM tasks WHERE jira_key = 'AP-42'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        phase3, "Reviewing",
        "after Coding (auto-complete), card should advance to Reviewing"
    );

    // -----------------------------------------------------------------------
    // Tick 4: Reviewing phase runs → card moves to Done (+ Jira transition)
    // -----------------------------------------------------------------------
    do_tick(&ctx).await.unwrap();
    wait_for_phase(&pool, "AP-42", "Done", 30).await;

    let (final_phase, _): (String, String) =
        sqlx::query_as("SELECT kanban_phase, phase_state FROM tasks WHERE jira_key = 'AP-42'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        final_phase, "Done",
        "after Reviewing, card should be in Done"
    );
}
