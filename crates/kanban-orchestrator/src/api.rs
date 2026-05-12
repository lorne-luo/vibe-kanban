use sqlx::SqlitePool;
use uuid::Uuid;

use crate::scheduler::ManualTrigger;

#[derive(Clone)]
pub struct KanbanHandle {
    trigger: ManualTrigger,
}

impl KanbanHandle {
    pub fn new(trigger: ManualTrigger) -> Self {
        Self { trigger }
    }

    pub fn poll_now(&self) {
        self.trigger.fire();
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RequestChangesBody {
    pub feedback: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TaskEventResponse {
    pub id: String,
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,
    pub ts: String,
}

pub async fn approve(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query("UPDATE tasks SET phase_state = 'idle', current_turn = 0 WHERE id = ?")
        .bind(id_bytes)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn request_changes(
    pool: &SqlitePool,
    task_id: Uuid,
    feedback: &str,
) -> crate::Result<()> {
    let payload = serde_json::json!({ "feedback": feedback }).to_string();
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query(
        "UPDATE tasks SET phase_state = 'running', current_turn = 0, pending_inject = ? WHERE id = ?",
    )
    .bind(payload)
    .bind(id_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cancel(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query("UPDATE tasks SET phase_state = 'archived' WHERE id = ?")
        .bind(id_bytes)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn retry_error(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    let id_bytes = task_id.as_bytes().to_vec();
    sqlx::query("UPDATE tasks SET phase_state = 'idle', error_info = NULL WHERE id = ?")
        .bind(id_bytes)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn events(pool: &SqlitePool, task_id: Uuid) -> crate::Result<Vec<TaskEventResponse>> {
    use sqlx::Row;
    let task_id_bytes = task_id.as_bytes().to_vec();
    let rows = sqlx::query(
        "SELECT id, event_type, from_phase, to_phase, actor, payload, ts \
         FROM task_events WHERE task_id = ? ORDER BY ts DESC",
    )
    .bind(task_id_bytes)
    .fetch_all(pool)
    .await?;

    let result = rows
        .iter()
        .map(|row| {
            let id_bytes: Vec<u8> = row.try_get("id").unwrap_or_default();
            let id_str = uuid::Uuid::from_slice(&id_bytes)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| "unknown".into());
            TaskEventResponse {
                id: id_str,
                event_type: row.try_get("event_type").unwrap_or_default(),
                from_phase: row.try_get("from_phase").ok().flatten(),
                to_phase: row.try_get("to_phase").ok().flatten(),
                actor: row.try_get("actor").unwrap_or_default(),
                payload: row.try_get("payload").ok().flatten(),
                ts: row.try_get("ts").unwrap_or_default(),
            }
        })
        .collect();
    Ok(result)
}
