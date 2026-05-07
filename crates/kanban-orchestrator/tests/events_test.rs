mod common;

#[tokio::test]
async fn emit_and_read_back() {
    let pool = common::test_pool().await;
    let proj_id = common::create_project(&pool, "test").await;
    // Create a task first (events have FK to tasks)
    let task_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, status, phase_state, current_turn, created_at, updated_at) VALUES (?, ?, 'test', 'todo', 'idle', 0, datetime('now'), datetime('now'))"
    )
    .bind(task_id.to_string())
    .bind(proj_id.to_string())
    .execute(&pool).await.unwrap();

    kanban_orchestrator::events::emit(&pool, task_id, "test_event", "test_actor", None, None, None)
        .await
        .unwrap();

    let events = db::models::task_event::TaskEvent::for_task(&pool, &task_id.to_string())
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "test_event");
}
