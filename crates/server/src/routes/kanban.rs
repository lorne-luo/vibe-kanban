use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json as ResponseJson,
    routing::{delete, post},
};
use deployment::Deployment;
use kanban_orchestrator::api::{approve, cancel_task, request_changes, retry_error};
use serde::{Deserialize, Serialize};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::DeploymentImpl;

#[derive(Debug, Deserialize)]
struct RequestChangesPayload {
    feedback: String,
}

#[derive(Debug, Serialize)]
struct KanbanStatus {
    enabled: bool,
}

async fn get_status(
    State(deployment): State<DeploymentImpl>,
) -> ResponseJson<ApiResponse<KanbanStatus>> {
    let enabled = deployment.kanban().is_enabled();
    ResponseJson(ApiResponse::success(KanbanStatus { enabled }))
}

async fn approve_task(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, StatusCode> {
    let pool = &deployment.db().pool;
    let handle = deployment.kanban();
    approve(pool, handle, task_id)
        .await
        .map(|_| ResponseJson(ApiResponse::success(())))
        .map_err(|e| {
            tracing::error!("kanban approve error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn changes_requested(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
    ResponseJson(payload): ResponseJson<RequestChangesPayload>,
) -> Result<ResponseJson<ApiResponse<()>>, StatusCode> {
    let pool = &deployment.db().pool;
    let handle = deployment.kanban();
    request_changes(pool, handle, task_id, &payload.feedback)
        .await
        .map(|_| ResponseJson(ApiResponse::success(())))
        .map_err(|e| {
            tracing::error!("kanban request_changes error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn archive_task(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, StatusCode> {
    let pool = &deployment.db().pool;
    cancel_task(pool, task_id)
        .await
        .map(|_| ResponseJson(ApiResponse::success(())))
        .map_err(|e| {
            tracing::error!("kanban cancel_task error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn retry_task(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, StatusCode> {
    let pool = &deployment.db().pool;
    let handle = deployment.kanban();
    retry_error(pool, handle, task_id)
        .await
        .map(|_| ResponseJson(ApiResponse::success(())))
        .map_err(|e| {
            tracing::error!("kanban retry_error error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// Immediately fire the scheduler tick for the given project.
/// The project_id path param is accepted for API consistency but the
/// orchestrator currently runs a single global tick; we simply fire the
/// ManualTrigger so it runs on the next scheduler loop iteration.
async fn poll_now(
    State(deployment): State<DeploymentImpl>,
    Path(_project_id): Path<Uuid>,
) -> ResponseJson<ApiResponse<()>> {
    deployment.kanban().trigger.fire();
    ResponseJson(ApiResponse::success(()))
}

/// Immediately re-sync a single task from Jira.  Like poll_now, this fires
/// the global trigger so the next tick re-reconciles all cards including the
/// requested one.  A task-scoped sync would require the full Jira client here;
/// firing the trigger is the minimal correct behaviour for now.
async fn sync_now(
    State(deployment): State<DeploymentImpl>,
    Path(_task_id): Path<Uuid>,
) -> ResponseJson<ApiResponse<()>> {
    deployment.kanban().trigger.fire();
    ResponseJson(ApiResponse::success(()))
}

pub(super) fn router(deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/kanban/status", axum::routing::get(get_status))
        .route("/kanban/tasks/{task_id}/approve", post(approve_task))
        .route(
            "/kanban/tasks/{task_id}/request-changes",
            post(changes_requested),
        )
        .route("/kanban/tasks/{task_id}/cancel", delete(archive_task))
        .route("/kanban/tasks/{task_id}/retry", post(retry_task))
        .route("/kanban/projects/{project_id}/poll_now", post(poll_now))
        .route("/kanban/tasks/{task_id}/sync_now", post(sync_now))
        .with_state(deployment.clone())
}
