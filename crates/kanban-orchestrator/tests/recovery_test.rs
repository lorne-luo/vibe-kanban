mod common;

use uuid::Uuid;

async fn insert_task(pool: &sqlx::SqlitePool, project_id: Uuid, phase_state: &str) -> Uuid {
    let id = Uuid::new_v4();
    let id_bytes = id.as_bytes().to_vec();
    let proj_bytes = project_id.as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, phase_state, current_turn)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id_bytes)
    .bind(proj_bytes)
    .bind(format!("task-{}", id))
    .bind(phase_state)
    .bind(3i64)
    .execute(pool)
    .await
    .expect("insert task");
    id
}

#[tokio::test]
async fn reset_running_resets_only_running_tasks() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "recovery-test").await;

    let running1 = insert_task(&pool, project_id, "running").await;
    let running2 = insert_task(&pool, project_id, "running").await;
    let awaiting = insert_task(&pool, project_id, "awaiting_review").await;
    let error_task = insert_task(&pool, project_id, "error").await;
    let idle_task = insert_task(&pool, project_id, "idle").await;

    let affected = kanban_orchestrator::recovery::reset_running(&pool)
        .await
        .expect("reset_running");

    assert_eq!(affected, 2, "two running tasks should be reset");

    // Check running tasks → idle, current_turn = 0
    for id in [running1, running2] {
        let (state, turn): (String, i64) = sqlx::query_as(
            "SELECT phase_state, current_turn FROM tasks WHERE id=?",
        )
        .bind(id.as_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .expect("fetch task");
        assert_eq!(state, "idle");
        assert_eq!(turn, 0);
    }

    // Verify other tasks are untouched
    let check = |id: Uuid, expected_state: &'static str| {
        let pool = pool.clone();
        async move {
            let (state, turn): (String, i64) = sqlx::query_as(
                "SELECT phase_state, current_turn FROM tasks WHERE id=?",
            )
            .bind(id.as_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .expect("fetch task");
            assert_eq!(state, expected_state);
            assert_eq!(turn, 3i64, "non-running tasks should keep original turn count");
        }
    };

    check(awaiting, "awaiting_review").await;
    check(error_task, "error").await;
    check(idle_task, "idle").await;
}

#[tokio::test]
async fn reset_running_returns_zero_when_no_running_tasks() {
    let pool = common::test_pool().await;
    let project_id = common::create_project(&pool, "no-running").await;
    insert_task(&pool, project_id, "idle").await;

    let affected = kanban_orchestrator::recovery::reset_running(&pool)
        .await
        .expect("reset_running");

    assert_eq!(affected, 0);
}
