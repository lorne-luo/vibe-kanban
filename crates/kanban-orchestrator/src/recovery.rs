use sqlx::SqlitePool;

/// Reset any tasks stuck in `running` back to `idle` after a crash or restart.
/// Tasks in `awaiting_review` or `error` are left untouched.
pub async fn reset_running(pool: &SqlitePool) -> crate::Result<u64> {
    let r = sqlx::query(
        "UPDATE tasks SET phase_state='idle', current_turn=0 WHERE phase_state='running'",
    )
    .execute(pool)
    .await?;
    tracing::info!(
        affected = r.rows_affected(),
        "startup recovery: reset interrupted running tasks to idle"
    );
    Ok(r.rows_affected())
}
