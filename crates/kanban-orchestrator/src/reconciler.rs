use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{config::Reconciliation, jira::JiraDiff};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconcileAction {
    Stop,
    QueueInject(String),
    Notify(String),
    UpdateSnapshot,
}

pub fn decide_action(diff: &JiraDiff, cfg: &Reconciliation) -> Vec<ReconcileAction> {
    let mut out = vec![ReconcileAction::UpdateSnapshot];
    if diff.status_terminal && cfg.on_jira_status_terminal.as_deref() == Some("stop_immediately") {
        out.push(ReconcileAction::Stop);
    }
    if diff.assignee_changed && cfg.on_jira_assignee_change.as_deref() == Some("stop_immediately") {
        out.push(ReconcileAction::Stop);
    }
    if !diff.new_comments.is_empty() && cfg.on_jira_comment.as_deref() == Some("inject_next_turn") {
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

pub struct UpsertOutcome {
    pub task_id: Uuid,
    pub created: bool,
    pub kanban_phase: Option<String>,
    pub diff: Option<crate::jira::JiraDiff>,
}

pub async fn upsert_card_from_jira(
    pool: &SqlitePool,
    project_id: Uuid,
    workflow: &crate::config::Workflow,
    issue: &crate::jira::JiraIssue,
) -> crate::Result<UpsertOutcome> {
    let snapshot_json = serde_json::to_string(issue).unwrap();
    let now = chrono::Utc::now();
    let now_str = now.to_rfc3339();

    // Look up existing by jira_key
    let project_id_bytes = project_id.as_bytes().to_vec();
    let row = sqlx::query("SELECT id, kanban_phase, jira_snapshot FROM tasks WHERE jira_key = ?")
        .bind(&issue.key)
        .fetch_optional(pool)
        .await?;

    if let Some(row) = row {
        use sqlx::Row;
        let id_bytes: Vec<u8> = row.try_get("id")?;
        let id = Uuid::from_slice(&id_bytes)
            .map_err(|e| crate::OrchestratorError::Other(anyhow::anyhow!(e)))?;
        let phase: Option<String> = row.try_get("kanban_phase")?;
        let prev_snap: Option<String> = row.try_get("jira_snapshot")?;
        let prev: crate::jira::JiraIssue = prev_snap
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_else(|| issue.clone());
        let diff = crate::jira::diff_issue(&prev, issue);
        let id_bytes_update = id.as_bytes().to_vec();
        sqlx::query("UPDATE tasks SET jira_snapshot = ?, jira_synced_at = ? WHERE id = ?")
            .bind(&snapshot_json)
            .bind(&now_str)
            .bind(id_bytes_update)
            .execute(pool)
            .await?;
        return Ok(UpsertOutcome {
            task_id: id,
            created: false,
            kanban_phase: phase,
            diff: Some(diff),
        });
    }

    // Create new card
    let id = Uuid::new_v4();
    let id_bytes = id.as_bytes().to_vec();
    // Prefer the column whose jira_status matches the issue's status; fall back
    // to the workflow's initial column when no mapping is configured.
    let initial = workflow
        .initial_column()
        .ok_or_else(|| crate::OrchestratorError::Workflow("no initial column".into()))?;
    let target_column = workflow
        .column_for_jira_status(&issue.fields.status.name)
        .unwrap_or(initial);
    let title = format!("{} {}", issue.key, issue.fields.summary);
    let description = issue
        .fields
        .description
        .as_ref()
        .map(crate::adf::to_markdown)
        .filter(|s| !s.is_empty());
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, description, status, jira_key, jira_snapshot, jira_synced_at, kanban_phase, phase_state, current_turn, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'todo', ?, ?, ?, ?, 'idle', 0, ?, ?)",
    )
    .bind(id_bytes)
    .bind(project_id_bytes)
    .bind(&title)
    .bind(description)
    .bind(&issue.key)
    .bind(&snapshot_json)
    .bind(&now_str)
    .bind(&target_column.name)
    .bind(&now_str)
    .bind(&now_str)
    .execute(pool)
    .await?;

    Ok(UpsertOutcome {
        task_id: id,
        created: true,
        kanban_phase: Some(target_column.name.clone()),
        diff: None,
    })
}

pub async fn enqueue_inject(pool: &SqlitePool, task_id: Uuid, blob: &str) -> crate::Result<()> {
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query("UPDATE tasks SET pending_inject = ? WHERE id = ?")
        .bind(blob)
        .bind(id_bytes)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_archived(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query("UPDATE tasks SET phase_state = 'archived' WHERE id = ?")
        .bind(id_bytes)
        .execute(pool)
        .await?;
    Ok(())
}
