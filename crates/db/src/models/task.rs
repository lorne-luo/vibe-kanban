use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use strum_macros::{Display, EnumString};
use ts_rs::TS;
use uuid::Uuid;

#[derive(
    Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS, EnumString, Display, Default,
)]
#[sqlx(type_name = "task_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Todo,
    InProgress,
    InReview,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PhaseState {
    Idle,
    Running,
    AwaitingReview,
    Error,
    Archived,
}

impl Default for PhaseState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Uuid, // Foreign key to Project
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub parent_workspace_id: Option<Uuid>, // Foreign key to parent Workspace
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub jira_key: Option<String>,
    pub jira_snapshot: Option<String>,
    pub jira_synced_at: Option<DateTime<Utc>>,
    pub kanban_phase: Option<String>,
    #[ts(type = "string")]
    pub phase_state: PhaseState,
    pub current_turn: i64,
    pub last_executor_session_id: Option<String>,
    pub review_pending_since: Option<DateTime<Utc>>,
    pub error_info: Option<String>,
    pub pending_inject: Option<String>,
}

impl Task {
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            Task,
            r#"SELECT
               id as "id!: Uuid",
               project_id as "project_id!: Uuid",
               title,
               description,
               status as "status!: TaskStatus",
               parent_workspace_id as "parent_workspace_id: Uuid",
               created_at as "created_at!: DateTime<Utc>",
               updated_at as "updated_at!: DateTime<Utc>",
               jira_key,
               jira_snapshot,
               jira_synced_at as "jira_synced_at: DateTime<Utc>",
               kanban_phase,
               phase_state as "phase_state!: PhaseState",
               current_turn as "current_turn!: i64",
               last_executor_session_id,
               review_pending_since as "review_pending_since: DateTime<Utc>",
               error_info,
               pending_inject
               FROM tasks
               ORDER BY created_at ASC"#
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            Task,
            r#"SELECT
               id as "id!: Uuid",
               project_id as "project_id!: Uuid",
               title,
               description,
               status as "status!: TaskStatus",
               parent_workspace_id as "parent_workspace_id: Uuid",
               created_at as "created_at!: DateTime<Utc>",
               updated_at as "updated_at!: DateTime<Utc>",
               jira_key,
               jira_snapshot,
               jira_synced_at as "jira_synced_at: DateTime<Utc>",
               kanban_phase,
               phase_state as "phase_state!: PhaseState",
               current_turn as "current_turn!: i64",
               last_executor_session_id,
               review_pending_since as "review_pending_since: DateTime<Utc>",
               error_info,
               pending_inject
               FROM tasks
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }
}
