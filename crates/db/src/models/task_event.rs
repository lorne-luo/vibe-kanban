use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,
    pub ts: DateTime<Utc>,
}

impl TaskEvent {
    pub async fn insert(pool: &SqlitePool, e: &TaskEvent) -> Result<(), sqlx::Error> {
        let id_bytes = e.id.as_bytes().to_vec();
        let task_id_bytes = e.task_id.as_bytes().to_vec();
        sqlx::query(
            "INSERT INTO task_events (id, task_id, event_type, from_phase, to_phase, actor, payload, ts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id_bytes)
        .bind(task_id_bytes)
        .bind(&e.event_type)
        .bind(&e.from_phase)
        .bind(&e.to_phase)
        .bind(&e.actor)
        .bind(&e.payload)
        .bind(e.ts.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn for_task(pool: &SqlitePool, task_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        let task_id_bytes = task_id.as_bytes().to_vec();
        let rows = sqlx::query(
            "SELECT id, task_id, event_type, from_phase, to_phase, actor, payload, ts \
             FROM task_events WHERE task_id = ? ORDER BY ts DESC",
        )
        .bind(task_id_bytes)
        .fetch_all(pool)
        .await?;

        rows.iter()
            .map(|row| {
                let id_bytes: Vec<u8> = row.try_get("id")?;
                let task_id_bytes: Vec<u8> = row.try_get("task_id")?;
                let ts_str: String = row.try_get("ts")?;
                Ok(TaskEvent {
                    id: Uuid::from_slice(&id_bytes)
                        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                    task_id: Uuid::from_slice(&task_id_bytes)
                        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                    event_type: row.try_get("event_type")?,
                    from_phase: row.try_get("from_phase")?,
                    to_phase: row.try_get("to_phase")?,
                    actor: row.try_get("actor")?,
                    payload: row.try_get("payload")?,
                    ts: ts_str
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                })
            })
            .collect()
    }
}
