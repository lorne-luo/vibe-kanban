use crate::config::Reconciliation;
use crate::jira::JiraDiff;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconcileAction {
    Stop,
    QueueInject(String),
    Notify(String),
    UpdateSnapshot,
}

pub fn decide_action(diff: &JiraDiff, cfg: &Reconciliation) -> Vec<ReconcileAction> {
    let mut out = vec![ReconcileAction::UpdateSnapshot];

    if diff.status_terminal
        && cfg.on_jira_status_terminal.as_deref() == Some("stop_immediately")
    {
        out.push(ReconcileAction::Stop);
    }
    if diff.assignee_changed
        && cfg.on_jira_assignee_change.as_deref() == Some("stop_immediately")
    {
        out.push(ReconcileAction::Stop);
    }
    if !diff.new_comments.is_empty()
        && cfg.on_jira_comment.as_deref() == Some("inject_next_turn")
    {
        let blob = format!(
            "New comments since last turn:\n{}",
            serde_json::to_string_pretty(&diff.new_comments).unwrap_or_default()
        );
        out.push(ReconcileAction::QueueInject(blob));
    }
    if (diff.summary_changed || diff.description_changed)
        && cfg.on_jira_summary_change.as_deref() == Some("inject_next_turn")
    {
        out.push(ReconcileAction::QueueInject(
            "Summary/description changed; re-read task.json.".into(),
        ));
    }
    if diff.status_changed {
        out.push(ReconcileAction::Notify("status_changed".into()));
    }
    out
}
