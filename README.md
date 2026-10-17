# Task Tracker

A small full-stack task manager for learning Rust: an **Axum** HTTP server with a
**SQLite** database and a plain **HTML/CSS/JavaScript** front end, all in one binary.

Tasks can be created, listed, edited, completed, returned to active and deleted.
Everything is stored in SQLite, so the list survives a restart.

---

## Table of contents

1. [Features](#features)
2. [Requirements](#requirements)
3. [Installation and running](#installation-and-running)
4. [Using the web interface](#using-the-web-interface)
5. [Running the tests](#running-the-tests)
6. [Project structure](#project-structure)
7. [HTTP API reference](#http-api-reference)
8. [Key design decisions](#key-design-decisions)
9. [Known limitations](#known-limitations)

---

## Features

| Feature | Where it happens |
| --- | --- |
| Main page with the task list | `GET /` |
| Create a task (title + optional description) | `POST /tasks` |
| View all tasks | `GET /tasks` |
| Edit a task | `PATCH /tasks/{id}` |
| Mark completed / return to active | `PATCH /tasks/{id}` with `{"completed": true|false}` |
| Delete a task | `DELETE /tasks/{id}` |
| Filters: all / active / completed | `GET /tasks?filter=all\|active\|completed` |
| Persistence in SQLite | `src/db.rs` |
| Validation with clear messages | `src/models.rs` + `src/error.rs` |
| Health check | `GET /health` |

---

## Requirements

* **Rust 1.85 or newer** — the crate uses the 2024 edition. (Developed and tested
  with Rust 1.99.)
* Nothing else. SQLite is compiled from source by the `rusqlite` crate
  (`bundled` feature), so there is no database to install and no service to start.

```powershell
rustc --version   # 1.85 or newer
```

---

## Installation and running

```powershell
git clone <repository-url>
cd RustWeb
cargo run
```

The server prints the address it is listening on:

```
Task Tracker running at http://127.0.0.1:3000
Using database tasks.db
```

Open <http://127.0.0.1:3000> in a browser.

The first run creates `tasks.db` in the working directory (plus
`tasks.db-journal` if a write is interrupted). Deleting that file resets the
application to an empty task list.

### Configuration

Both settings are optional and are read from environment variables. They contain
paths and port numbers, never secrets.

| Variable | Default | Meaning |
| --- | --- | --- |
| `PORT` | `3000` | TCP port to listen on. `127.0.0.1` only. |
| `DATABASE_PATH` | `tasks.db` | SQLite file. `:memory:` uses a throwaway database. |

```powershell
# Another port and a database in a different place
$env:PORT = "8080"
$env:DATABASE_PATH = "C:\data\tasks.db"
cargo run

# Throwaway database: everything is lost when the server stops
$env:DATABASE_PATH = ":memory:"
cargo run
```

### Release build

```powershell
cargo build --release
.\target\release\rust_task_tracker.exe
```

---

## Using the web interface

1. Type a title in **New task**, optionally add a description, press **Add task**.
2. Use the checkbox on the left of a task to complete it or make it active again.
3. **Edit** turns the row into a small form; **Save** stores the change, **Cancel** discards it.
4. **Delete** asks for confirmation and then removes the task.
5. **All / Active / Completed** reload the list from the server using the
   `?filter=` query parameter.
6. Errors reported by the server (for example an empty title) appear in the status
   line above the list, exactly as the API returned them.

---

## Running the tests

```powershell
cargo test
```

The suite has two levels:

* **Unit tests** inside `src/models.rs` (validation and filter parsing) and
  `src/db.rs` (SQLite round-trips, filters, partial updates) — 13 tests.
* **Integration tests** in `tests/api.rs` that drive the real router through
  `tower::ServiceExt::oneshot` — 19 tests covering the endpoints, filters,
  validation, error codes, the served UI and persistence across a reopen of the
  database file.

Each test creates its own in-memory database, so tests are independent and can run
in parallel. No port is bound during testing.

Other useful commands:

```powershell
cargo fmt --check     # formatting
cargo check           # fast type check of every target
cargo test -- --nocapture   # show output printed by tests
```

---

## Project structure

```
Cargo.toml              Dependencies (axum, tokio, serde, rusqlite)
README.md               This file
POST_TASKS.md           Design note written before this stage was implemented

src/
  main.rs               Binary entry point: configuration, database, listener
  lib.rs                Module wiring + `app(db)`, the router (single source of truth)
  models.rs             Task, request bodies, filters, and all input validation
  db.rs                 SQLite: schema, CRUD, error type
  handlers.rs           HTTP handlers, one small function per endpoint
  error.rs              AppError -> HTTP status + {"error": "..."} body
  assets.rs             Serves the UI, embedded with include_str!
  web/
    index.html          The single page
    style.css           Styles, no framework
    app.js              Front-end logic (fetch + DOM, no framework)

tests/
  api.rs                HTTP-level integration tests
```

The layers only depend downwards: `handlers` → `db` → `models`. Everything that
can fail is expressed as a `Result`, and the SQLite work runs on a blocking thread
pool so the async runtime is never blocked on disk I/O.

---

## HTTP API reference

All responses are JSON, except the UI files. Errors use the same shape everywhere:

```json
{ "error": "title must not be empty" }
```

### `GET /health`

```json
{ "status": "ok" }
```

### `GET /tasks`

Optional query parameter `filter`: `all` (default), `active` or `completed`.

```json
[
  { "id": 1, "title": "Buy milk", "description": "Two litres", "completed": false }
]
```

### `POST /tasks`

```json
{ "title": "Buy milk", "description": "Two litres" }
```

* `title` — required, trimmed, 1–200 characters.
* `description` — optional, trimmed, up to 2000 characters. Missing, `null` or
  empty means "no description".
* `completed` — optional, defaults to `false`.

Answers `201 Created` with the stored task, including the assigned `id`. Tasks are
listed in the order they were created.

### `PATCH /tasks/{id}`

Send only the fields you want to change; everything else keeps its value.

```json
{ "completed": true }
{ "title": "Buy oat milk" }
{ "description": "" }
```

An empty `description` **clears** it. Answers `200 OK` with the updated task, or
`404` if the id does not exist.

### `DELETE /tasks/{id}`

Answers `204 No Content` with an empty body, or `404` if the id does not exist.

### Status codes

| Status | When |
| --- | --- |
| `200 OK` | Successful `GET` / `PATCH` |
| `201 Created` | Task created |
| `204 No Content` | Task deleted |
| `400 Bad Request` | Body is not valid JSON, or `{id}` is not a number |
| `404 Not Found` | Unknown task id, or unknown route |
| `415 Unsupported Media Type` | Missing `Content-Type: application/json` header |
| `422 Unprocessable Entity` | Validation failed (empty title, unknown filter, ...) |
| `500 Internal Server Error` | Storage failure; details go to the server's stderr, not the client |

### Command-line examples

`curl.exe` is used explicitly because PowerShell aliases `curl` to
`Invoke-WebRequest`. Passing the JSON in a file avoids the shell's quote handling.

```powershell
curl.exe http://127.0.0.1:3000/health
curl.exe http://127.0.0.1:3000/tasks
curl.exe "http://127.0.0.1:3000/tasks?filter=active"

# Create a task
'{"title":"Buy milk","description":"Two litres"}' | Out-File -Encoding utf8NoBOM body.json
curl.exe -X POST http://127.0.0.1:3000/tasks -H "Content-Type: application/json" --data-binary "@body.json"

# Complete it, then delete it
curl.exe -X PATCH http://127.0.0.1:3000/tasks/1 -H "Content-Type: application/json" -d '{\"completed\":true}'
curl.exe -X DELETE http://127.0.0.1:3000/tasks/1
```

---

## Key design decisions

**One SQLite connection behind a `Mutex`, queries in `spawn_blocking`.**
`rusqlite` is a blocking library. Shared state is `Arc<Mutex<Connection>>`, and each
query runs inside `tokio::task::spawn_blocking` so disk waits never stall the async
worker threads. A pool would be the next step for a multi-user server; for a local
single-user application one connection is simpler and fully sufficient.

**Ids are `i64`, not `u64`.** SQLite's `INTEGER` is a *signed* 64-bit value.
`AUTOINCREMENT` guarantees ids are never reused, even after the newest task is
deleted.

**`PATCH` instead of `PUT`.** The UI needs to flip the `completed` flag without
sending the title. `PATCH` with optional fields matches that, and "field absent"
is distinguishable from "field set to empty" thanks to `Option<Option<String>>`
for the description (see the comment in `src/models.rs`).

**Filters are bound parameters, not string concatenation.**
`GET /tasks?filter=active` becomes `WHERE (?1 IS NULL OR completed = ?1)`. The
filter value travels as a query parameter, so user input never becomes SQL. Every
other statement is parameterised the same way.

**The UI is embedded with `include_str!`.** The HTML, CSS and JS are compiled into
the binary: no `tower-http` static-file service, no working-directory assumptions,
and the server cannot start without its assets. The assets are served from the same
origin as the API, so no CORS configuration is needed.

**The front end writes text with `textContent`, never `innerHTML`.** A task titled
`<script>alert(1)</script>` is displayed as text. This is the one XSS rule that
actually matters for this app.

**`GET /health` stays a pure liveness probe.** It returns `{"status":"ok"}` without
touching the database, exactly as before. If a request is answered at all, the
process and the async runtime are working.

**No `unwrap` or `expect` anywhere in the crate.** Failures travel as
`Result<_, AppError>` (HTTP) or `Result<_, DbError>` (storage). `main` returns
`Result<(), Box<dyn Error>>`. The only `panic!` is in a test helper that should
never trigger.

---

## Known limitations

* Single user, local only: the server binds `127.0.0.1`, with no authentication or
  authorization.
* One database connection, so writes are serialised. Fine for one user.
* Assets are embedded at compile time, so editing `src/web/*` requires a rebuild.
* No logging framework: internal errors are printed to stderr, and there is no
  request log or `tracing` integration.
* No pagination; `GET /tasks` always returns the whole list.
* The schema is created with `CREATE TABLE IF NOT EXISTS`, not a versioned
  migration system, so changing a column later needs a manual migration.
* No graceful shutdown handling: the process exits when it is stopped.
