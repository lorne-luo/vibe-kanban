//! Event emission helpers — persist structured events to task_events table.

use db::models::task_event::{NewTaskEvent, TaskEvent};
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn emit(
    pool: &SqlitePool,
    task_id: Uuid,
    event_type: &str,
    actor: &str,
    from_phase: Option<&str>,
    to_phase: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> crate::Result<()> {
    let e = NewTaskEvent {
        id: Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        event_type: event_type.to_string(),
        from_phase: from_phase.map(String::from),
        to_phase: to_phase.map(String::from),
        actor: actor.to_string(),
        payload: payload.map(|v| v.to_string()),
        ts: chrono::Utc::now().to_rfc3339(),
    };
    TaskEvent::insert(pool, &e).await?;
    Ok(())
}
