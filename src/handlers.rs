//! HTTP handlers for the JSON API.
//!
//! Each handler is deliberately thin: validate the input, call the storage
//! layer, return a status code and a JSON body. Failures are reported through
//! [`AppError`], which owns the mapping to HTTP.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::db::Database;
use crate::error::AppError;
use crate::models::{Health, ListQuery, NewTask, Task, TaskPatch};

/// `GET /health` — liveness probe.
///
/// Returns `200 OK` and `{"status":"ok"}` whenever the process can answer.
pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".to_owned(),
    })
}

/// `GET /tasks` — lists tasks.
///
/// The optional `?filter=` query parameter accepts `all` (default), `active` or
/// `completed`. An unknown value is rejected with `422`.
pub async fn list_tasks(
    State(db): State<Database>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Task>>, AppError> {
    let filter = query.filter()?;
    let tasks = db.list(filter.completed_flag()).await?;

    Ok(Json(tasks))
}

/// `POST /tasks` — creates a task and answers `201 Created`.
pub async fn create_task(
    State(db): State<Database>,
    // A body-consuming extractor must be the last argument.
    Json(input): Json<NewTask>,
) -> Result<(StatusCode, Json<Task>), AppError> {
    let task = db.create(input.validate()?).await?;

    Ok((StatusCode::CREATED, Json(task)))
}

/// `PATCH /tasks/{id}` — updates the fields present in the body.
///
/// Fields that are absent keep their stored value, so the UI can flip
/// `completed` without sending the title. Answers `404` for an unknown id.
pub async fn update_task(
    State(db): State<Database>,
    Path(id): Path<i64>,
    Json(input): Json<TaskPatch>,
) -> Result<Json<Task>, AppError> {
    let task = db.update(id, input.validate()?).await?;

    task.map(Json).ok_or(AppError::NotFound)
}

/// `DELETE /tasks/{id}` — deletes a task and answers `204 No Content`.
///
/// Answers `404` when the id is unknown, rather than reporting a successful
/// delete of something that was never there.
pub async fn delete_task(
    State(db): State<Database>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    if db.delete(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}
