use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory pool");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

#[tokio::test]
async fn kanban_columns_exist() {
    let pool = test_pool().await;
    let row =
        sqlx::query("SELECT COUNT(*) as cnt FROM pragma_table_info('tasks') WHERE name='jira_key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let cnt: i64 = row.get(0);
    assert_eq!(cnt, 1, "jira_key column should exist");

    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM sqlite_master WHERE type='table' AND name='task_events'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let cnt: i64 = row.get(0);
    assert_eq!(cnt, 1, "task_events table should exist");
}
