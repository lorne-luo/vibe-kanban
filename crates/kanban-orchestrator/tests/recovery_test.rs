mod common;
use uuid::Uuid;

#[tokio::test]
async fn reset_running_resets_to_idle() {
    let pool = common::test_pool().await;
    let proj_id = common::create_project(&pool, "rec").await;
    let task_id = Uuid::new_v4();

    // Insert a task in 'running' state
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, phase_state, current_turn, created_at, updated_at) \
         VALUES (?, ?, 'test', 'todo', 'running', 3, datetime('now'), datetime('now'))"
    )
    .bind(task_id.to_string())
    .bind(proj_id.to_string())
    .execute(&pool).await.unwrap();

    let affected = kanban_orchestrator::recovery::reset_running(&pool)
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Verify the card is now idle with turn=0
    let row: (String, i64) =
        sqlx::query_as("SELECT phase_state, current_turn FROM tasks WHERE id=?")
            .bind(task_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, "idle");
    assert_eq!(row.1, 0);
}

#[tokio::test]
async fn reset_running_leaves_awaiting_review_untouched() {
    let pool = common::test_pool().await;
    let proj_id = common::create_project(&pool, "rec2").await;
    let task_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, phase_state, current_turn, created_at, updated_at) \
         VALUES (?, ?, 'test', 'todo', 'awaiting_review', 3, datetime('now'), datetime('now'))"
    )
    .bind(task_id.to_string())
    .bind(proj_id.to_string())
    .execute(&pool).await.unwrap();

    kanban_orchestrator::recovery::reset_running(&pool)
        .await
        .unwrap();

    let row: (String,) = sqlx::query_as("SELECT phase_state FROM tasks WHERE id=?")
        .bind(task_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(row.0, "awaiting_review");
}
