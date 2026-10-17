//! The web UI served by the same process as the API.
//!
//! The three files are embedded into the binary with `include_str!`, so the
//! server works from any working directory, needs no static-file service
//! dependency and cannot be started "without its assets".

use axum::http::header;
use axum::response::{Html, IntoResponse, Response};

/// The single page that makes up the UI.
pub const INDEX_HTML: &str = include_str!("web/index.html");

/// Stylesheet of the UI.
const STYLE_CSS: &str = include_str!("web/style.css");

/// Front-end logic.
const APP_JS: &str = include_str!("web/app.js");

/// `GET /` — the main page.
pub async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// `GET /style.css` — the stylesheet.
pub async fn stylesheet() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

/// `GET /app.js` — the front-end script.
pub async fn script() -> Response {
    asset("text/javascript; charset=utf-8", APP_JS)
}

/// Builds an asset response with an explicit content type.
///
/// `no-cache` stops the browser from showing a stale asset after a rebuild,
/// which is what a developer editing these files wants.
fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}
