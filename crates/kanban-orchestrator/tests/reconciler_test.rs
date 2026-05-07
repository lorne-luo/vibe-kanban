mod common;

use kanban_orchestrator::{
    config::Reconciliation,
    jira::{JiraComment, JiraDiff},
    reconciler::{ReconcileAction, decide_action},
};

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
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::QueueInject(_))),
        "expected QueueInject action"
    );
}

#[test]
fn always_has_update_snapshot() {
    let d = JiraDiff::default();
    let cfg = Reconciliation::default();
    let actions = decide_action(&d, &cfg);
    assert!(actions.contains(&ReconcileAction::UpdateSnapshot));
}

#[tokio::test]
async fn new_issue_creates_card_in_initial_column() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "AP").await;

    let workflow_yaml = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    let workflow: kanban_orchestrator::config::Workflow =
        serde_yaml::from_str(&workflow_yaml).unwrap();

    let issue = kanban_orchestrator::jira::JiraIssue {
        key: "AP-42".into(),
        fields: kanban_orchestrator::jira::JiraFields {
            summary: "Test card".into(),
            status: kanban_orchestrator::jira::JiraNamed {
                name: "To Do".into(),
            },
            assignee: None,
            description: None,
            labels: vec![],
            priority: None,
            comment: kanban_orchestrator::jira::JiraComments { comments: vec![] },
            attachment: vec![],
        },
    };

    let outcome = kanban_orchestrator::reconciler::upsert_card_from_jira(
        &pool, project_id, &workflow, &issue,
    )
    .await
    .unwrap();

    assert!(outcome.created);
    assert_eq!(outcome.kanban_phase, Some("Todo".to_string()));
}
