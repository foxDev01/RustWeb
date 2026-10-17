//! Integration tests for the Task Tracker HTTP API.
//!
//! Each test drives the real router through `tower::ServiceExt::oneshot`, so no
//! socket is bound. Every test builds its own in-memory database, which makes the
//! tests independent of each other and of their execution order.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use rust_task_tracker::{Database, app};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Largest response body the tests are willing to read.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Builds the application on top of a fresh, empty in-memory database.
fn test_app() -> Result<Router, Box<dyn std::error::Error>> {
    Ok(app(Database::open_in_memory()?))
}

/// Sends a request through the router and returns the status plus the decoded body.
///
/// A JSON body is asserted to be declared as `application/json`; anything else
/// (for example a plain-text rejection produced by an extractor before the
/// handler runs) is returned as a JSON string so the status can still be tested.
async fn request(
    router: Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let mut builder = Request::builder().method(method).uri(uri);
    let request_body = match body {
        Some(json) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&json)?)
        }
        None => Body::empty(),
    };

    // `oneshot` consumes the service, which is why callers pass `router.clone()`.
    let response = router.oneshot(builder.body(request_body)?).await?;
    let status = response.status();

    if status == StatusCode::NO_CONTENT {
        return Ok((status, Value::Null));
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    let bytes = to_bytes(response.into_body(), MAX_BODY_BYTES).await?;
    let text = String::from_utf8_lossy(&bytes).into_owned();

    match serde_json::from_str::<Value>(&text) {
        Ok(value) => {
            assert_eq!(
                content_type, "application/json",
                "a JSON body must be declared as application/json ({uri})"
            );
            Ok((status, value))
        }
        Err(_) => Ok((status, Value::String(text))),
    }
}

/// `GET uri`.
async fn get(router: Router, uri: &str) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    request(router, Method::GET, uri, None).await
}

/// `POST uri` with a JSON body.
async fn post(
    router: Router,
    uri: &str,
    body: Value,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    request(router, Method::POST, uri, Some(body)).await
}

/// `PATCH uri` with a JSON body.
async fn patch(
    router: Router,
    uri: &str,
    body: Value,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    request(router, Method::PATCH, uri, Some(body)).await
}

/// `DELETE uri`.
async fn delete(
    router: Router,
    uri: &str,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    request(router, Method::DELETE, uri, None).await
}

/// Creates a task and returns the created representation.
async fn create_task(router: &Router, title: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let (status, body) = post(router.clone(), "/tasks", json!({ "title": title })).await?;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "creating `{title}` should succeed"
    );

    Ok(body)
}

// ---------------------------------------------------------------------------
// Endpoints from the first version of the project: their behaviour is unchanged.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_returns_status_ok() -> Result<(), Box<dyn std::error::Error>> {
    let (status, body) = get(test_app()?, "/health").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "ok" }));

    Ok(())
}

#[tokio::test]
async fn list_tasks_returns_empty_json_array() -> Result<(), Box<dyn std::error::Error>> {
    let (status, body) = get(test_app()?, "/tasks").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    Ok(())
}

#[tokio::test]
async fn unknown_route_returns_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let (status, _) = get(test_app()?, "/does-not-exist").await?;

    assert_eq!(status, StatusCode::NOT_FOUND);

    Ok(())
}

// ---------------------------------------------------------------------------
// Creating tasks.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_returns_the_stored_task() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    let (status, body) = post(
        router,
        "/tasks",
        json!({ "title": "Buy milk", "description": "Two litres" }),
    )
    .await?;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        body,
        json!({ "id": 1, "title": "Buy milk", "description": "Two litres", "completed": false })
    );

    Ok(())
}

#[tokio::test]
async fn created_task_appears_in_the_list() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    create_task(&router, "Buy milk").await?;

    let (status, body) = get(router, "/tasks").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!([{ "id": 1, "title": "Buy milk", "description": null, "completed": false }])
    );

    Ok(())
}

#[tokio::test]
async fn create_task_trims_whitespace_and_defaults_description()
-> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    let (status, body) = post(
        router,
        "/tasks",
        json!({ "title": "  padded  ", "description": "   " }),
    )
    .await?;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["title"], json!("padded"));
    assert_eq!(
        body["description"],
        json!(null),
        "blank text means no description"
    );
    assert_eq!(body["completed"], json!(false));

    Ok(())
}

#[tokio::test]
async fn create_task_accepts_an_explicit_completed_flag() -> Result<(), Box<dyn std::error::Error>>
{
    let router = test_app()?;

    let (status, body) = post(
        router,
        "/tasks",
        json!({ "title": "Already done", "completed": true }),
    )
    .await?;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["completed"], json!(true));

    Ok(())
}

#[tokio::test]
async fn create_task_rejects_invalid_input() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    // Empty after trimming.
    let (status, body) = post(router.clone(), "/tasks", json!({ "title": "   " })).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json!({ "error": "title must not be empty" }));

    // Longer than the 200 character limit.
    let (status, body) = post(
        router.clone(),
        "/tasks",
        json!({ "title": "a".repeat(201) }),
    )
    .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("at most 200"),
        "unexpected message: {body}"
    );

    // Nothing was stored by the rejected requests.
    let (status, body) = get(router, "/tasks").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    Ok(())
}

#[tokio::test]
async fn create_task_requires_a_valid_json_body() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    // Missing required field -> axum rejects the body before the handler runs.
    let (status, _) = post(router.clone(), "/tasks", json!({})).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Wrong type for a field.
    let (status, _) = post(router.clone(), "/tasks", json!({ "title": 42 })).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // No Content-Type: application/json header at all.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/tasks")
                .body(Body::from(r#"{"title":"no header"}"#))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    Ok(())
}

#[tokio::test]
async fn malformed_json_is_a_bad_request() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    // Valid content type, invalid JSON syntax -> 400 (not 422).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title": "unterminated"#))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

// ---------------------------------------------------------------------------
// Listing and filtering.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tasks_returns_tasks_in_creation_order() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    create_task(&router, "first").await?;
    create_task(&router, "second").await?;

    let (status, body) = get(router, "/tasks").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["title"], json!("first"));
    assert_eq!(body[1]["title"], json!("second"));
    assert_eq!(body.as_array().map(Vec::len), Some(2));

    Ok(())
}

#[tokio::test]
async fn list_tasks_filters_by_state() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    let active = create_task(&router, "active task").await?;
    let created = create_task(&router, "completed task").await?;

    // Mark the second task as done. The response is the stored task, so it is the
    // value to compare the filtered list against.
    let (status, completed) = patch(
        router.clone(),
        &format!("/tasks/{}", created["id"]),
        json!({ "completed": true }),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completed["completed"], json!(true));

    let (status, body) = get(router.clone(), "/tasks?filter=all").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().map(Vec::len), Some(2));

    let (status, body) = get(router.clone(), "/tasks?filter=active").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([active]));

    let (status, body) = get(router, "/tasks?filter=completed").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([completed]));

    Ok(())
}

#[tokio::test]
async fn list_tasks_rejects_an_unknown_filter() -> Result<(), Box<dyn std::error::Error>> {
    let (status, body) = get(test_app()?, "/tasks?filter=everything").await?;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "error": "unknown filter `everything`; expected `all`, `active` or `completed`" })
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Updating tasks.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_updates_only_the_fields_it_receives() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    let created = create_task(&router, "original").await?;
    let uri = format!("/tasks/{}", created["id"]);

    // Change the title only: description and completion are untouched.
    let (status, body) = patch(router.clone(), &uri, json!({ "title": "renamed" })).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], json!("renamed"));
    assert_eq!(body["description"], json!(null));
    assert_eq!(body["completed"], json!(false));

    // Mark it as completed, which is what the checkbox in the UI sends.
    let (status, body) = patch(router.clone(), &uri, json!({ "completed": true })).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["title"],
        json!("renamed"),
        "the title is still the new one"
    );
    assert_eq!(body["completed"], json!(true));

    // Return it to the active state.
    let (status, body) = patch(router.clone(), &uri, json!({ "completed": false })).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["completed"], json!(false));

    // An empty description clears it; adding text sets it again.
    let (status, _) = patch(router.clone(), &uri, json!({ "description": "details" })).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = patch(router.clone(), &uri, json!({ "description": "" })).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["description"], json!(null));

    Ok(())
}

#[tokio::test]
async fn patch_rejects_invalid_input_and_unknown_ids() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    let created = create_task(&router, "keep me").await?;
    let uri = format!("/tasks/{}", created["id"]);

    let (status, body) = patch(router.clone(), &uri, json!({ "title": "" })).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json!({ "error": "title must not be empty" }));

    let (status, body) = patch(router.clone(), "/tasks/9999", json!({ "title": "ghost" })).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "task not found" }));

    // The rejected update changed nothing.
    let (_, body) = get(router, "/tasks").await?;
    assert_eq!(body[0]["title"], json!("keep me"));

    Ok(())
}

#[tokio::test]
async fn patch_with_a_non_numeric_id_is_a_bad_request() -> Result<(), Box<dyn std::error::Error>> {
    let (status, _) = patch(test_app()?, "/tasks/abc", json!({ "title": "x" })).await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);

    Ok(())
}

// ---------------------------------------------------------------------------
// Deleting tasks.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_removes_the_task() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;
    let created = create_task(&router, "temporary").await?;
    let uri = format!("/tasks/{}", created["id"]);

    let (status, body) = delete(router.clone(), &uri).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::Null, "204 responses carry no body");

    let (status, body) = get(router.clone(), "/tasks").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));

    // Deleting it twice reports the second attempt as missing.
    let (status, body) = delete(router, &uri).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "task not found" }));

    Ok(())
}

// ---------------------------------------------------------------------------
// The web UI and persistence.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_main_page_and_assets_are_served() -> Result<(), Box<dyn std::error::Error>> {
    let router = test_app()?;

    for (uri, marker) in [
        ("/", "Task Tracker"),
        ("/style.css", ".task"),
        ("/app.js", "/tasks"),
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty())?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK, "{uri} should be served");

        let bytes = to_bytes(response.into_body(), MAX_BODY_BYTES).await?;
        let content = String::from_utf8_lossy(&bytes);
        assert!(content.contains(marker), "{uri} should contain `{marker}`");
    }

    Ok(())
}

#[tokio::test]
async fn tasks_survive_reopening_the_database() -> Result<(), Box<dyn std::error::Error>> {
    // A real file is needed here: the point of the test is that data outlives
    // the connection. The name includes the process id so parallel runs of this
    // binary cannot collide.
    let path = std::env::temp_dir().join(format!(
        "task_tracker_persistence_test_{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path); // clear leftovers from an earlier failed run

    {
        let router = app(Database::open(&path)?);
        let (status, _) = post(
            router,
            "/tasks",
            json!({ "title": "persisted", "description": "survives a restart" }),
        )
        .await?;
        assert_eq!(status, StatusCode::CREATED);
    } // the connection is dropped here, exactly as it is when the server stops

    let router = app(Database::open(&path)?);
    let (status, body) = get(router, "/tasks").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!([{
            "id": 1,
            "title": "persisted",
            "description": "survives a restart",
            "completed": false
        }])
    );

    // Best-effort cleanup: on Windows a file may still be held briefly, and a
    // failed cleanup must not fail the test.
    let _ = std::fs::remove_file(&path);

    Ok(())
}
