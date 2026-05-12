use chrono::Utc;
use db::models::task::{PhaseState, Task, TaskStatus};
use uuid::Uuid;

fn sample_card() -> Task {
    Task {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        title: "Test task".into(),
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
        current_turn: 2,
        last_executor_session_id: None,
        review_pending_since: None,
        error_info: None,
        pending_inject: None,
    }
}

#[test]
fn writes_task_json_phase_md() {
    let dir = tempfile::tempdir().unwrap();
    let card = sample_card();
    kanban_orchestrator::context::write_context(dir.path(), &card, Some("reconcile-blob"), None)
        .unwrap();
    let task = std::fs::read_to_string(dir.path().join(".kanban-context/task.json")).unwrap();
    assert!(
        task.contains("\"jira_key\""),
        "task.json should contain jira_key"
    );
    let rec = std::fs::read_to_string(dir.path().join(".kanban-context/reconcile.md")).unwrap();
    assert_eq!(rec, "reconcile-blob");
    assert!(dir.path().join(".kanban-context/phase.md").exists());
}
