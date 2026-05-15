use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use deployment::Deployment;
use kanban_orchestrator::api::{self, RequestChangesBody, TaskEventResponse};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub async fn poll_now(
    State(deployment): State<DeploymentImpl>,
    Path(_project_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    kanban_orchestrator::runtime::trigger_poll_now(deployment.db()).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn approve(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::approve(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn request_changes(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
    Json(body): Json<RequestChangesBody>,
) -> Result<StatusCode, ApiError> {
    api::request_changes(&deployment.db().pool, task_id, &body.feedback).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn cancel(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::cancel(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn retry_error(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::retry_error(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn events(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<Json<ApiResponse<Vec<TaskEventResponse>>>, ApiError> {
    let evts = api::events(&deployment.db().pool, task_id).await?;
    Ok(Json(ApiResponse::success(evts)))
}

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/projects/{project_id}/kanban/poll-now", post(poll_now))
        .route("/tasks/{task_id}/kanban/approve", post(approve))
        .route(
            "/tasks/{task_id}/kanban/request-changes",
            post(request_changes),
        )
        .route("/tasks/{task_id}/kanban/cancel", post(cancel))
        .route("/tasks/{task_id}/kanban/retry-error", post(retry_error))
        .route("/tasks/{task_id}/kanban/events", get(events))
}
