use kanban_orchestrator::{
    config::Workflow,
    jira::{JiraComment, JiraComments, JiraFields, JiraIssue, JiraNamed},
};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use uuid::Uuid;

pub async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory pool");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();

    // Create projects table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projects (
            id            BLOB PRIMARY KEY,
            name          TEXT NOT NULL,
            git_repo_path TEXT NOT NULL DEFAULT '' UNIQUE,
            setup_script  TEXT DEFAULT '',
            created_at    TEXT NOT NULL DEFAULT (datetime('now','subsec')),
            updated_at    TEXT NOT NULL DEFAULT (datetime('now','subsec'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Create tasks table with all kanban orchestrator columns
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tasks (
            id                      BLOB PRIMARY KEY,
            project_id              BLOB NOT NULL,
            title                   TEXT NOT NULL,
            description             TEXT,
            status                  TEXT NOT NULL DEFAULT 'todo'
                                        CHECK (status IN ('todo','inprogress','done','cancelled','inreview')),
            parent_workspace_id     BLOB,
            created_at              TEXT NOT NULL DEFAULT (datetime('now','subsec')),
            updated_at              TEXT NOT NULL DEFAULT (datetime('now','subsec')),
            jira_key                TEXT,
            jira_snapshot           TEXT,
            jira_synced_at          TEXT,
            kanban_phase            TEXT,
            phase_state             TEXT NOT NULL DEFAULT 'idle'
                                        CHECK (phase_state IN ('idle','running','awaiting_review','error','archived')),
            current_turn            INTEGER NOT NULL DEFAULT 0,
            last_executor_session_id BLOB,
            review_pending_since    TEXT,
            error_info              TEXT,
            pending_inject          TEXT,
            FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_jira_key ON tasks(jira_key) WHERE jira_key IS NOT NULL",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Create task_events table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_events (
            id         BLOB PRIMARY KEY,
            task_id    BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            event_type TEXT NOT NULL,
            from_phase TEXT,
            to_phase   TEXT,
            actor      TEXT NOT NULL,
            payload    TEXT,
            ts         TEXT NOT NULL DEFAULT (datetime('now','subsec'))
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

pub async fn create_project(pool: &SqlitePool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let id_bytes = id.as_bytes().to_vec();
    sqlx::query("INSERT INTO projects (id, name, git_repo_path) VALUES (?, ?, ?)")
        .bind(id_bytes)
        .bind(name)
        .bind(format!("/tmp/test/{}", name))
        .execute(pool)
        .await
        .expect("create project");
    id
}

pub fn load_test_workflow() -> Workflow {
    let yml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/valid_workflow.yml"
    ))
    .expect("valid_workflow.yml");
    serde_yaml::from_str(&yml).expect("parse workflow")
}

pub fn sample_issue() -> JiraIssue {
    JiraIssue {
        key: "AP-1".into(),
        fields: JiraFields {
            summary: "Test issue".into(),
            status: JiraNamed {
                name: "To Do".into(),
            },
            assignee: None,
            description: None,
            labels: vec![],
            priority: None,
            comment: JiraComments { comments: vec![] },
            attachment: vec![],
        },
    }
}

pub fn sample_comment(id: &str) -> JiraComment {
    JiraComment {
        id: id.into(),
        body: serde_json::Value::Null,
        updated: "2026-05-07T00:00:00Z".into(),
    }
}
