//! The single error type returned by HTTP handlers.
//!
//! Handlers return `Result<_, AppError>`, and `AppError` knows how to turn
//! itself into an HTTP response: a status code plus a small JSON body of the
//! shape `{"error": "..."}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::db::DbError;
use crate::models::ValidationError;

/// Everything a handler can fail with.
#[derive(Debug)]
pub enum AppError {
    /// The request body or query string was invalid (`422`).
    Validation(ValidationError),
    /// The requested task does not exist (`404`).
    NotFound,
    /// Storage failed (`500`). The details are logged, never returned.
    Database(DbError),
}

impl From<ValidationError> for AppError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        Self::Database(error)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => write!(formatter, "invalid request: {error}"),
            Self::NotFound => formatter.write_str("task not found"),
            Self::Database(error) => write!(formatter, "storage failure: {error}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::NotFound => None,
            Self::Database(error) => Some(error),
        }
    }
}

/// The JSON body sent for every failed request.
#[derive(Debug, Serialize)]
struct ErrorBody {
    /// Human-readable explanation, safe to display in a UI.
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Validation(error) => {
                (StatusCode::UNPROCESSABLE_ENTITY, error.message().to_owned())
            }
            Self::NotFound => (StatusCode::NOT_FOUND, "task not found".to_owned()),
            Self::Database(error) => {
                // No logging framework is used in this project, so the internal
                // detail goes to stderr for the developer running the server.
                // It is deliberately not part of the response body.
                eprintln!("database error: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal storage error".to_owned(),
                )
            }
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}
