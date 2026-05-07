use std::sync::Arc;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{config::Workflow, scheduler::ManualTrigger};

/// Handle exposed to the HTTP layer for controlling the orchestrator
#[derive(Clone)]
pub struct KanbanHandle {
    pub trigger: ManualTrigger,
    pub workflow: Option<Arc<Workflow>>, // None if orchestrator disabled
}

impl KanbanHandle {
    pub fn disabled() -> Self {
        KanbanHandle {
            trigger: ManualTrigger::new(),
            workflow: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.workflow.is_some()
    }
}

pub async fn approve(pool: &SqlitePool, handle: &KanbanHandle, task_id: Uuid) -> crate::Result<()> {
    // Advance the card by clearing awaiting_review state and triggering a reschedule tick.
    // The tick will pick it up and re-dispatch if it's in an agent column.
    sqlx::query(
        "UPDATE tasks SET phase_state='idle', current_turn=0, review_pending_since=NULL \
         WHERE id=? AND phase_state='awaiting_review'",
    )
    .bind(task_id.to_string())
    .execute(pool)
    .await?;
    handle.trigger.fire();
    Ok(())
}

/// Sentinel prefix written into `pending_inject` when the content is review
/// feedback rather than a Jira-reconcile blob.  `tick.rs` inspects this prefix
/// to route the value to `PhaseInputs::review_feedback` instead of
/// `PhaseInputs::reconcile_blob`.
pub(crate) const REVIEW_FEEDBACK_PREFIX: &str = "##REVIEW_FEEDBACK##\n";

pub async fn request_changes(
    pool: &SqlitePool,
    handle: &KanbanHandle,
    task_id: Uuid,
    feedback: &str,
) -> crate::Result<()> {
    // Prefix the blob so the dispatcher routes it to review_feedback.md, not
    // reconcile.md.
    let blob = format!(
        "{}# Review Feedback\n\n{}",
        REVIEW_FEEDBACK_PREFIX, feedback
    );
    sqlx::query(
        "UPDATE tasks SET phase_state='idle', current_turn=0, pending_inject=? \
         WHERE id=? AND phase_state='awaiting_review'",
    )
    .bind(&blob)
    .bind(task_id.to_string())
    .execute(pool)
    .await?;
    handle.trigger.fire();
    Ok(())
}

pub async fn cancel_task(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET phase_state='archived' WHERE id=?")
        .bind(task_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn retry_error(
    pool: &SqlitePool,
    handle: &KanbanHandle,
    task_id: Uuid,
) -> crate::Result<()> {
    sqlx::query(
        "UPDATE tasks SET phase_state='idle', current_turn=0, error_info=NULL \
         WHERE id=? AND phase_state='error'",
    )
    .bind(task_id.to_string())
    .execute(pool)
    .await?;
    handle.trigger.fire();
    Ok(())
}
