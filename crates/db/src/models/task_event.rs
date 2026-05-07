use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: String,      // UUID stored as text
    pub task_id: String, // UUID stored as text
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,
    pub ts: String, // RFC3339 stored as text
}

impl TaskEvent {
    pub async fn insert(pool: &SqlitePool, e: &NewTaskEvent) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO task_events (id, task_id, event_type, from_phase, to_phase, actor, payload, ts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            e.id, e.task_id, e.event_type, e.from_phase, e.to_phase, e.actor, e.payload, e.ts
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn for_task(pool: &SqlitePool, task_id: &str) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            TaskEvent,
            r#"SELECT
               id as "id!: String",
               task_id as "task_id!: String",
               event_type as "event_type!: String",
               from_phase,
               to_phase,
               actor as "actor!: String",
               payload,
               ts as "ts!: String"
               FROM task_events WHERE task_id = ? ORDER BY ts DESC"#,
            task_id
        )
        .fetch_all(pool)
        .await
    }
}

pub struct NewTaskEvent {
    pub id: String,
    pub task_id: String,
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,
    pub ts: String,
}
