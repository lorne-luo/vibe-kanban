mod common;

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use kanban_orchestrator::{
    api,
    dispatcher::{PhaseExecutor, TurnOutcome, gate::Gate},
    jira::JiraClient,
    notifier::Notifier,
    scheduler::tick::{OrchestratorContext, do_tick},
};
use sqlx::Row;
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

/// Executor whose Nth turn emits COMPLETE (0-indexed call count across all turns).
struct ScriptedExecutor {
    call_num: AtomicUsize,
    complete_on: Vec<usize>,
}

impl ScriptedExecutor {
    fn new(complete_on: Vec<usize>) -> Self {
        Self {
            call_num: AtomicUsize::new(0),
            complete_on,
        }
    }
}

#[async_trait]
impl PhaseExecutor for ScriptedExecutor {
    async fn run_turn(
        &self,
        _: &Path,
        _: &Path,
        _: Duration,
    ) -> kanban_orchestrator::Result<TurnOutcome> {
        let n = self.call_num.fetch_add(1, Ordering::SeqCst);
        let stdout = if self.complete_on.contains(&n) {
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

fn make_e2e_workflow(jira_base: &str) -> kanban_orchestrator::config::Workflow {
    let yml = format!(
        r#"
version: 1
project: E2E
sync:
  source: jira
  jira:
    site: {jira_base}
    project_key: E2E
    jql: "project = E2E"
    poll_interval: 1h
    auth_env:
      email: E2E_JIRA_EMAIL
      token: E2E_JIRA_TOKEN
columns:
  - name: Coding
    jira_status: ["To Do"]
    initial: true
    agent: coder
    on_complete: review
    next: Reviewing
    max_turns_per_phase: 1
  - name: Reviewing
    agent: reviewer
    on_complete: auto
    next: Done
    jira_transition: Done
    max_turns_per_phase: 1
  - name: Done
    jira_status: ["Done"]
    terminal: true
"#
    );
    serde_yaml::from_str(&yml).expect("parse e2e workflow")
}

async fn fetch_task_state(pool: &sqlx::SqlitePool, task_id: Uuid) -> (String, String) {
    let row = sqlx::query("SELECT phase_state, kanban_phase FROM tasks WHERE id = ?")
        .bind(task_id.as_bytes().to_vec())
        .fetch_one(pool)
        .await
        .unwrap();
    let phase_state: String = row.try_get("phase_state").unwrap();
    let kanban_phase: String = row.try_get::<Option<String>, _>("kanban_phase").unwrap().unwrap_or_default();
    (phase_state, kanban_phase)
}

/// Full e2e: Jira poll → create card → dispatch coder (awaiting_review) →
/// approve → dispatch coder (complete) → advance to Reviewing →
/// dispatch reviewer (complete) → advance to Done + Jira transition called.
#[tokio::test]
async fn e2e_poll_dispatch_review_done() {
    let server = MockServer::start().await;

    // Jira search always returns AP-E2E-1 in "To Do"
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issues": [{
                "key": "AP-E2E-1",
                "fields": {
                    "summary": "E2E Test Issue",
                    "status": { "name": "To Do" },
                    "assignee": null,
                    "comment": { "comments": [] },
                    "description": null,
                    "labels": [],
                    "priority": null,
                    "attachment": []
                }
            }]
        })))
        .mount(&server)
        .await;

    // Transitions list for jira_transition calls
    Mock::given(method("GET"))
        .and(path_regex(r"/rest/api/3/issue/.+/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "transitions": [{"id": "31", "name": "Done"}]
        })))
        .mount(&server)
        .await;

    // Transition POST expected exactly once (when Reviewing → Done)
    Mock::given(method("POST"))
        .and(path_regex(r"/rest/api/3/issue/.+/transitions"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "E2E").await;
    let workflow = make_e2e_workflow(&server.uri());

    let repo_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo_root.path().join(".agents/agent")).unwrap();
    for name in &["coder", "reviewer"] {
        std::fs::write(
            repo_root.path().join(format!(".agents/agent/{name}.md")),
            format!("---\nname: {name}\n---\nDo the work"),
        )
        .unwrap();
    }

    // call 0 → "still working" (coder, turn 1): max_turns=1, on_complete=review → AwaitingReview
    // call 1 → COMPLETE (coder after approve, turn 1) → advance to Reviewing
    // call 2 → COMPLETE (reviewer, turn 1) → advance to Done + jira_transition
    let executor = Arc::new(ScriptedExecutor::new(vec![1, 2]));

    let ctx = OrchestratorContext {
        pool: pool.clone(),
        workflow,
        repo_root: repo_root.path().to_path_buf(),
        jira: Arc::new(JiraClient::new(server.uri(), "test@e2e".into(), "tok".into())),
        gate: Arc::new(Gate::new(5, 2)),
        executor,
        notifier: Arc::new(Notifier::new()),
        project_id,
    };

    // ── Tick 1: reconcile creates card, coder exhausts turns → awaiting_review ──
    do_tick(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (task_id_bytes,): (Vec<u8>,) =
        sqlx::query_as("SELECT id FROM tasks WHERE jira_key = 'AP-E2E-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let task_id = Uuid::from_slice(&task_id_bytes).unwrap();

    let (state, phase) = fetch_task_state(&pool, task_id).await;
    assert_eq!(phase, "Coding", "new card should start in Coding (initial column)");
    assert_eq!(state, "awaiting_review", "coder exhausts 1 turn with on_complete=review");

    // ── Approve ──
    api::approve(&pool, task_id).await.unwrap();
    let (state, _) = fetch_task_state(&pool, task_id).await;
    assert_eq!(state, "idle", "approved card should be idle");

    // ── Tick 2: coder emits COMPLETE → advance to Reviewing ──
    do_tick(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (state, phase) = fetch_task_state(&pool, task_id).await;
    assert_eq!(phase, "Reviewing", "coder complete should advance to Reviewing");
    assert_eq!(state, "idle");

    // ── Tick 3: reviewer emits COMPLETE → advance to Done + jira_transition ──
    do_tick(&ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (state, phase) = fetch_task_state(&pool, task_id).await;
    assert_eq!(phase, "Done", "reviewer complete should advance to Done");
    assert_eq!(state, "idle");

    // wiremock verify: transition POST was called exactly once
    server.verify().await;
}
