use sqlx::SqlitePool;
use uuid::Uuid;

/// Emit a task lifecycle event.
/// Stub implementation — G2 will persist to the task_events table.
pub async fn emit(
    _pool: &SqlitePool,
    _task_id: Uuid,
    _event_type: &str,
    _actor: &str,
    _from_phase: Option<&str>,
    _to_phase: Option<&str>,
    _payload: Option<&serde_json::Value>,
) -> crate::Result<()> {
    // Stub — Task G2 will implement persistent event storage
    Ok(())
}
