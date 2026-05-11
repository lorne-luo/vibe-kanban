use db::DBService;
use uuid::Uuid;
use std::sync::OnceLock;

use crate::scheduler::ManualTrigger;

static KANBAN_TRIGGER: OnceLock<ManualTrigger> = OnceLock::new();

pub async fn trigger_poll_now(_db: &DBService) -> crate::Result<()> {
    if let Some(trigger) = KANBAN_TRIGGER.get() {
        trigger.fire();
    }
    Ok(())
}

pub async fn sync_task_now(_db: &DBService, _task_id: Uuid) -> crate::Result<()> {
    Ok(())
}

pub async fn poll_once(db: &DBService, project: String) -> crate::Result<()> {
    tracing::info!(project, "running one kanban poll tick");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&db.pool)
        .await?;
    tracing::info!(project, task_count = count, "poll_once completed (stub)");
    Ok(())
}

pub fn set_trigger(trigger: ManualTrigger) {
    let _ = KANBAN_TRIGGER.set(trigger);
}
