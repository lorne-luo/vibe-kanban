use kanban_orchestrator::config::Reconciliation;
use kanban_orchestrator::jira::{JiraComment, JiraDiff};
use kanban_orchestrator::reconciler::{decide_action, ReconcileAction};

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
        actions.iter().any(|a| matches!(a, ReconcileAction::QueueInject(_))),
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
