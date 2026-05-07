use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;

pub async fn test_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .pragma("journal_mode", "WAL");
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    // Run all migrations (path relative to CARGO_MANIFEST_DIR = crates/kanban-orchestrator)
    sqlx::migrate!("../db/migrations")
        .run(&pool)
        .await
        .unwrap();
    pool
}

pub async fn create_project(pool: &SqlitePool, name: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let id_str = id.to_string();
    // After migrations, projects table: id BLOB, name TEXT, remote_project_id BLOB,
    // created_at TEXT, updated_at TEXT  (git_repo_path removed in 20251209 migration)
    sqlx::query(
        "INSERT INTO projects (id, name, created_at, updated_at) \
         VALUES (?, ?, datetime('now'), datetime('now'))",
    )
    .bind(&id_str)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    id
}
