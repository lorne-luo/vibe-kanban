use kanban_orchestrator::{
    config::Reconciliation,
    jira::{JiraComment, JiraDiff},
    reconciler::{ReconcileAction, decide_action},
};

mod common;

#[test]
fn terminal_status_stops_immediately() {
    let mut d = JiraDiff::default();
    d.status_changed = true;
    d.status_terminal = true;
    let cfg = Reconciliation {
        on_jira_status_terminal: Some("stop_immediately".into()),
        ..Default::default()
    };
    let actions = decide_action(&d, &cfg);
    assert!(actions.contains(&ReconcileAction::Stop));
    assert!(actions.contains(&ReconcileAction::Notify("status_changed".into())));
}

#[test]
fn new_comment_queues_inject() {
    let mut d = JiraDiff::default();
    d.new_comments.push(JiraComment {
        id: "c1".into(),
        body: serde_json::Value::Null,
        updated: "2026-05-07T00:00:00Z".into(),
    });
    let cfg = Reconciliation {
        on_jira_comment: Some("inject_next_turn".into()),
        ..Default::default()
    };
    let actions = decide_action(&d, &cfg);
    assert!(matches!(
        actions.first(),
        Some(ReconcileAction::UpdateSnapshot)
    ));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::QueueInject(_)))
    );
}

#[tokio::test]
async fn new_issue_creates_card_in_initial_column() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "AP").await;
    let workflow = common::load_test_workflow();
    let issue = common::sample_issue();
    let outcome = kanban_orchestrator::reconciler::upsert_card_from_jira(
        &pool, project_id, &workflow, &issue,
    )
    .await
    .unwrap();
    assert!(outcome.created);
    assert_eq!(outcome.kanban_phase, Some("Todo".to_string()));
}

#[tokio::test]
async fn second_upsert_returns_diff_not_created() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "AP2").await;
    let workflow = common::load_test_workflow();
    let mut issue = common::sample_issue();
    issue.key = "AP-2".into();

    // First upsert creates the card
    let first = kanban_orchestrator::reconciler::upsert_card_from_jira(
        &pool, project_id, &workflow, &issue,
    )
    .await
    .unwrap();
    assert!(first.created);

    // Modify the issue and upsert again
    issue.fields.summary = "Updated summary".into();
    let second = kanban_orchestrator::reconciler::upsert_card_from_jira(
        &pool, project_id, &workflow, &issue,
    )
    .await
    .unwrap();
    assert!(!second.created);
    assert!(second.diff.is_some());
    let diff = second.diff.unwrap();
    assert!(diff.summary_changed);
}
