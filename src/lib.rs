//! Task Tracker: a small full-stack task manager used for learning Rust.
//!
//! The crate is split into layers that only depend on the layer below:
//!
//! * [`models`] — the data and all input validation;
//! * [`db`] — SQLite storage;
//! * [`handlers`] — the HTTP API;
//! * [`assets`] — the embedded web UI;
//! * [`error`] — how failures become HTTP responses.
//!
//! The router lives here rather than in `main.rs` so that the binary and the
//! integration tests build exactly the same application.

pub mod assets;
pub mod db;
pub mod error;
pub mod handlers;
pub mod models;

use axum::Router;
use axum::routing::{get, patch};

pub use db::Database;

/// Builds the application router.
///
/// Every route the API exposes is registered here, so adding an endpoint means
/// adding one `.route(...)` call and nothing else. `db` becomes the axum state
/// that the handlers receive.
pub fn app(db: Database) -> Router {
    Router::new()
        // Web UI. Embedded in the binary, served from the same origin as the
        // API so the browser needs no CORS configuration.
        .route("/", get(assets::index))
        .route("/style.css", get(assets::stylesheet))
        .route("/app.js", get(assets::script))
        // JSON API. `/health` and `/tasks` keep the behaviour of the first
        // version of this project; `/tasks` now stores what it returns.
        .route("/health", get(handlers::health))
        .route(
            "/tasks",
            get(handlers::list_tasks).post(handlers::create_task),
        )
        .route(
            "/tasks/{id}",
            patch(handlers::update_task).delete(handlers::delete_task),
        )
        .with_state(db)
}
