use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use db::models::task_event::TaskEvent;

pub async fn emit(
    pool: &SqlitePool,
    task_id: Uuid,
    event_type: &str,
    actor: &str,
    from_phase: Option<&str>,
    to_phase: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> crate::Result<()> {
    let row = TaskEvent {
        id: Uuid::new_v4(),
        task_id,
        event_type: event_type.to_string(),
        from_phase: from_phase.map(str::to_string),
        to_phase: to_phase.map(str::to_string),
        actor: actor.to_string(),
        payload: payload.map(|v| v.to_string()),
        ts: Utc::now(),
    };
    TaskEvent::insert(pool, &row).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE tasks (id BLOB PRIMARY KEY, project_id BLOB NOT NULL, \
             title TEXT NOT NULL, description TEXT, status TEXT NOT NULL DEFAULT 'todo', \
             parent_workspace_id BLOB, \
             created_at TEXT NOT NULL DEFAULT (datetime('now','subsec')), \
             updated_at TEXT NOT NULL DEFAULT (datetime('now','subsec')), \
             jira_key TEXT, jira_snapshot TEXT, jira_synced_at TEXT, \
             kanban_phase TEXT, phase_state TEXT NOT NULL DEFAULT 'idle', \
             current_turn INTEGER NOT NULL DEFAULT 0, \
             last_executor_session_id BLOB, review_pending_since TEXT, \
             error_info TEXT, pending_inject TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE task_events (id BLOB PRIMARY KEY, \
             task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             event_type TEXT NOT NULL, from_phase TEXT, to_phase TEXT, \
             actor TEXT NOT NULL, payload TEXT, \
             ts TEXT NOT NULL DEFAULT (datetime('now','subsec')))",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn emits_and_reads_back() {
        let pool = test_pool().await;
        let task_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tasks (id, project_id, title, status, created_at, updated_at) \
             VALUES (?, ?, 'Test', 'todo', datetime('now'), datetime('now'))",
        )
        .bind(task_id.as_bytes().to_vec())
        .bind(project_id.as_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();

        emit(
            &pool,
            task_id,
            "phase_advanced",
            "dispatcher",
            Some("Coding"),
            Some("Reviewing"),
            None,
        )
        .await
        .unwrap();

        let events = db::models::task_event::TaskEvent::for_task(&pool, task_id)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "phase_advanced");
        assert_eq!(events[0].from_phase.as_deref(), Some("Coding"));
    }
}
