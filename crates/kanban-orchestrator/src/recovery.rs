//! Startup recovery — reset any tasks that were left in `running` state when
//! the orchestrator was previously interrupted.

use sqlx::SqlitePool;

/// Reset all tasks with `phase_state = 'running'` back to `idle` and reset
/// their turn counter to 0. Returns the number of rows affected.
pub async fn reset_running(pool: &SqlitePool) -> crate::Result<u64> {
    let result = sqlx::query(
        "UPDATE tasks SET phase_state='idle', current_turn=0 WHERE phase_state='running'",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
