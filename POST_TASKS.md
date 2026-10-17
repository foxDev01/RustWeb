# Design note: `POST /tasks` (historical)

> **Status: historical — already implemented.** This note was written *before* the
> endpoint existed, so it describes the plan as it stood then. It is kept for
> reference; see [`README.md`](README.md) for what the project does today.
>
> Two things changed during implementation:
>
> * storage is **SQLite** (`src/db.rs`), not the in-memory
>   `Arc<RwLock<Vec<Task>>>` sketched below — persisting tasks was a later
>   requirement;
> * the code was split into `models.rs` / `db.rs` / `handlers.rs` / `error.rs`
>   instead of growing inside `lib.rs`.
>
> The rest of the document is still an accurate description of how the endpoint
> behaves: `POST /tasks` answers `201` with the stored task, and the rejection
> codes are as listed at the end.

Original plan, written when the API had only `GET /health` and `GET /tasks`:

| Endpoint | Method | Status at that time |
| --- | --- | --- |
| `/health` | `GET` | implemented, returned `{"status":"ok"}` |
| `/tasks` | `GET` | implemented, returned `[]` |
| `/tasks` | `POST` | **planned in this document** — returned `405` then |

---

## Table of contents

1. [Files that need to change](#1-files-that-need-to-change)
2. [`Arc<RwLock<Vec<Task>>>` explained](#2-arcrwlockvectask-explained)
3. [The request and response JSON](#3-the-request-and-response-json)
4. [How to verify with tests](#4-how-to-verify-with-tests)
5. [Errors that can arise and how to avoid them](#5-errors-that-can-arise-and-how-to-avoid-them)
6. [Appendices: full target code and commands](#6-appendices-full-target-code-and-commands)

---

## 1. Files that need to change

| File | Change | Size of change |
| --- | --- | --- |
| `src/lib.rs` | Add `AppState` (the shared store), `CreateTask` input type, the `create_task` handler, an `AppError` type, and register `.post(create_task)` on the existing `/tasks` route. | ~80 lines added |
| `src/main.rs` | **Nothing.** | 0 lines |
| `tests/api.rs` | Add tests for create + list-after-create, and the rejection cases. | ~60 lines added |
| `Cargo.toml` | **Nothing** — no new dependencies needed. | 0 lines |

### Why `main.rs` does not change

`main.rs` only knows `rust_task_tracker::app()`. As long as `app()` keeps returning `Router`
(the fully-stated router) and builds the store internally, the binary is unaffected. That is the
payoff of the earlier `lib.rs` / `main.rs` split:

```rust
// main.rs — stays exactly as it is
let listener = TcpListener::bind(addr).await?;
axum::serve(listener, app()).await?;
```

The doc comment on `lib.rs` already promises this ("The router lives here rather than in `main.rs`
so that the binary and the integration tests build exactly the same application").

### The shape of the change in `src/lib.rs`

```rust
// 1. a type alias so the state type is readable in signatures
pub type Store = Arc<RwLock<Vec<Task>>>;

// 2. input type for POST bodies
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTask {
    pub title: String,
    #[serde(default)]
    pub completed: bool,
}

// 3. the router grows one method handler and one state
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/tasks", get(list_tasks).post(create_task)) // <- add .post(...)
        .with_state(Store::default())                       // <- provide the state
}
```

`Store::default()` works because `Arc<RwLock<Vec<Task>>>` implements `Default`
(`Arc<T>: Default` when `T: Default`, `RwLock<T>: Default` when `T: Default`).

---

## 2. `Arc<RwLock<Vec<Task>>>` explained

Read it inside-out, one layer at a time:

```
Arc<  RwLock<  Vec<Task>  >  >
 |       |        |
 |       |        +-- the actual data: a growable list of tasks
 |       +----------- a lock: "many readers OR one writer, never both"
 +------------------- shared ownership: several owners of the same lock
```

### Layer 1: `Vec<Task>` — the data

Plain data, no concurrency concerns. Nothing new.

### Layer 2: `RwLock<Vec<Task>>` — interior mutability

**The problem it solves:** your handlers receive the state by *shared* reference, not by
`&mut`. Look at a handler signature — there is no `mut` anywhere:

```rust
async fn create_task(State(store): State<Store>, Json(input): Json<CreateTask>) -> ...
//                         ^^^^^^^^^^^ this is a shared (immutable) handle
```

In Rust you normally need `&mut Vec<Task>` to call `push`. A `lock` gives you that `&mut`
*through* an immutable reference. This trick has a name: **interior mutability**.

A tiny, single-threaded example of the same idea with `RefCell` (the non-threadsafe cousin of
`RwLock`) makes it concrete:

```rust
use std::cell::RefCell;

struct Counter {
    value: RefCell<u32>, // interior mutability
}

impl Counter {
    fn bump(&self) {                 // note: &self, not &mut self
        *self.value.borrow_mut() += 1;
    }
}

let counter = Counter { value: RefCell::new(0) };
counter.bump();
counter.bump();
assert_eq!(*counter.value.borrow(), 2);
```

`RwLock` is `RefCell` for multiple threads, and it makes the borrow rules explicit:

| Call | Gives you | Blocks when |
| --- | --- | --- |
| `store.read()` | a read guard → `&Vec<Task>` | another thread holds the write guard |
| `store.write()` | a write guard → `&mut Vec<Task>` | any other reader or writer is active |

`GET /tasks` only reads, so many `GET`s can run concurrently — that is the "Rw" (read-write)
advantage over a plain `Mutex`, which would serialise everything.

### Layer 3: `Arc<...>` — shared ownership across tasks

`RwLock<Vec<Task>>` alone cannot be given to more than one handler: Rust has a single owner per
value, and moving it into one request handler would leave nothing for the next request. `Arc`
("**A**tomic **r**eference **c**ounted") is a pointer that can be cloned cheaply, where every
clone points at the *same* value, and the value is dropped when the last `Arc` disappears.

Smallest useful example:

```rust
use std::sync::{Arc, RwLock};

fn main() {
    let store: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));

    let handle = Arc::clone(&store); // a second owner of the SAME lock, not a copy of the data
    std::thread::spawn(move || {
        handle.write().unwrap().push("written by another thread".to_owned());
    })
    .join()
    .unwrap();

    assert_eq!(store.read().unwrap().len(), 1);
}
```

Three points to notice:

1. `Arc::clone(&store)` copies the *pointer*. `(*store).clone()` would deep-copy the `Vec` and is
   almost never what you want.
2. `move ||` moves the cloned handle into the closure, so the lock outlives the function body
   that created it — exactly the situation with request handlers.
3. `Arc` is needed because axum handlers run on different Tokio worker **tasks**, which may be
   scheduled onto different OS threads. Thread-safety is enforced at compile time: `Router` state
   must be `Send + Sync + 'static`, and `Arc<RwLock<Vec<Task>>>` satisfies all three.

### Why not something else?

| Alternative | Why it is not used here |
| --- | --- |
| `Mutex` | Works, compiles, and is simpler — but serialises readers. `RwLock` teaches the read/write distinction and costs nothing here. |
| `tokio::sync::RwLock` | The async counterpart. Needed only if you must hold the lock **across an `.await`** (e.g. waiting on the network). Our handlers do no I/O while holding the lock, so the cheaper `std::sync::RwLock` is the better fit. See [§5.2](#52-holding-a-lock-across-an-await). |
| A global `static` | Requires `OnceLock`/`lazy_static` and hides the dependency. Passing state through the router is the idiomatic axum way and keeps tests isolated. |
| A database | Out of scope for this stage, and the API surface (`Store`) would not change much when you add one. |

### Key generation: where do `id`s come from?

`CreateTask` comes from the client and has no `id` — the server assigns it. The simplest correct
approach is to compute the next id **while holding the write lock**, so two concurrent `POST`s
cannot receive the same id:

```rust
let mut tasks = store.write().map_err(|_| AppError::LockPoisoned)?;
let id = tasks.iter().map(|task| task.id).max().map_or(1, |max| max + 1);
```

`Option::map_or(1, |max| max + 1)` reads as "if the list is empty, start at 1, otherwise
`max + 1`" and does not panic. (`unwrap_or` would also be fine here — it is not the panicking
`unwrap()`. Both are fine; the project rule is about not calling `unwrap()` on `Result`s.)

An alternative is an `AtomicU64` counter in the state:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

let next_id = AtomicU64::new(1);
let id = next_id.fetch_add(1, Ordering::Relaxed); // 1, 2, 3, ... even if tasks are deleted
```

This is the better choice **later**, once `DELETE /tasks/{id}` exists and ids must never be
reused. For now, "max + 1 while holding the lock" needs no second field, so it stays simpler.

---

## 3. The request and response JSON

### Request

```http
POST /tasks HTTP/1.1
Content-Type: application/json
```

```json
{
  "title": "Buy milk"
}
```

Because `CreateTask.completed` is annotated `#[serde(default)]`, the field is optional and
defaults to `false`. Both of these are therefore valid:

```json
{ "title": "Buy milk", "completed": false }
```

```json
{ "title": "Buy milk" }
```

`id` must **not** be sent by the client: it is not a field of `CreateTask`, so serde simply
ignores it. (If you would rather reject unknown fields loudly, add
`#[serde(deny_unknown_fields)]` to `CreateTask` — then `{"id": 5, ...}` becomes a `422`.)

### Response — `201 Created`

```json
{
  "id": 1,
  "title": "Buy milk",
  "completed": false
}
```

The echo of the stored task (including the server-assigned `id`) is what lets a client know what
was created. Optionally add a `Location: /tasks/1` header; say the word if you want that.

### Subsequent `GET /tasks`

```json
[
  {
    "id": 1,
    "title": "Buy milk",
    "completed": false
  }
]
```

### Status codes

| Situation | Status | Body |
| --- | --- | --- |
| Task created | `201` | the created task |
| Title missing / wrong type / malformed JSON | `400` or `422` | axum's rejection text (see [§5.4](#54-extractor-rejections-happen-before-your-handler)) |
| Missing `Content-Type: application/json` | `415` | axum's rejection text |
| Title present but empty / whitespace | `422` | `{"error":"title must not be empty"}` |
| Lock poisoned by a panic | `500` | `{"error":"internal storage error"}` |

### Manual check with curl

```powershell
curl.exe -i -X POST http://127.0.0.1:3000/tasks `
  -H "Content-Type: application/json" `
  -d '{\"title\":\"Buy milk\"}'

curl.exe http://127.0.0.1:3000/tasks
```

(In PowerShell, `curl` is an alias for `Invoke-WebRequest` — always write `curl.exe`. The
backslash-escaped quotes are needed because PowerShell parses the single-quoted string's inner
quotes itself; a here-string or `--data-raw @body.json` is often less painful.)

---

## 4. How to verify with tests

All of this belongs in the existing `tests/api.rs`, reusing the `oneshot` technique already in
place.

### 4.1 The one change the existing test helpers need

Today `get_json()` calls `app()` internally, so each request gets a **brand-new, empty store**.
That is perfect for isolation but wrong for a test that creates a task and then lists tasks.
Generalise the helper to accept a router:

```rust
async fn get_json(router: Router, path: &str) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let response = router
        .oneshot(Request::builder().uri(path).body(Body::empty())?)
        .await?;
    // ... unchanged from the current implementation
}
```

`oneshot` **consumes** the service, which is why the router must be `.clone()`d for each request —
`axum::Router` is cheap to clone (it is internally an `Arc`). Callers then read:

```rust
let router = app();
let (_, first) = get_json(router.clone(), "/health").await?;
let (_, second) = get_json(router.clone(), "/tasks").await?;
```

The existing three tests just add `app()` at the call site. Because `app()` still creates a private
store per call, **each test remains isolated and order-independent** — no test can see another
test's tasks, and tests can run in parallel.

A POST helper:

```rust
async fn post_json(
    router: Router,
    path: &str,
    body: &str,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_owned()))?,
        )
        .await?;
    // status + content-type assertion + JSON decode, same as get_json
}
```

### 4.2 Tests worth writing

| Test | What it pins down |
| --- | --- |
| `create_task_returns_created_task` | `201`, and the body is `{"id":1,"title":"Buy milk","completed":false}`. |
| `created_task_appears_in_list` | POST then GET on the same router → the array has exactly that one task. This is the test that proves the store is actually shared. |
| `two_tasks_get_distinct_ids` | The id generator is not off-by-one or duplicated. |
| `create_task_defaults_completed_to_false` | `{"title":"x"}` omits `completed` and still yields `false` — proves `#[serde(default)]`. |
| `create_task_rejects_empty_title` | `422` and `{"error":"title must not be empty"}`; also assert `GET /tasks` is still empty (nothing partially stored). |
| `create_task_rejects_missing_title` | `422` from axum's serde rejection. |
| `create_task_requires_json_content_type` | `415` when the header is absent — a very common client mistake. |
| `create_task_rejects_malformed_json` | `400` for `{"title":` (syntax error, not a type error). |
| `list_tasks_is_empty_on_a_fresh_app` | The existing `GET /tasks` test still passes — guards against accidental global state. |

### 4.3 Why not unit-test the handler function directly

You could call `create_task(State(store), Json(input))` as a plain function. It is valid, but
integration tests through `oneshot` also cover routing, the `State` extractor wiring, the JSON
content-type contract, the status code and the serialised output — i.e. everything a real client
depends on. Keep the integration tests as the primary safety net.

### 4.4 A quick sanity check on the response struct

For the body assertion, `serde_json::json!` is exact and readable:

```rust
assert_eq!(body, serde_json::json!({ "id": 1, "title": "Buy milk", "completed": false }));
```

Note that `serde_json::Value` object comparison ignores key order but **array** comparison does
not: `json!([1, 2]) != json!([2, 1])`. For the list test, prefer asserting the length and the
first element, or build the expected array explicitly.

---

## 5. Errors that can arise and how to avoid them

### 5.1 Forgetting `.with_state(...)`, or calling it too early

`Router::new()` is a `Router<()>`; adding a handler that asks for `State<Store>` turns it into a
`Router<Store>`, which is **not yet a valid server**: `axum::serve` needs a `Router<()>`. The
error message mentions missing state or `with_state`, e.g. *"the trait bound `Store: FromRef<()>`
is not satisfied"*.

Rules of thumb:

```rust
Router::new()
    .route("/tasks", get(list_tasks).post(create_task)) // 1. add routes first
    .with_state(store)                                  // 2. provide state last
```

Because `app()` is the only place that builds the router, the mistake can only happen in one
function — another reason to keep routing in `lib.rs`.

### 5.2 Holding a lock across an `.await`

This is the single most confusing `RwLock` error you can hit. `std::sync::RwLockReadGuard` is
**not** `Send`, so if the guard is alive at an `.await`, the whole handler future stops being
`Send` and axum rejects the handler:

```
error: future cannot be sent between threads safely
   = help: within `impl Future`, the trait `Send` is not implemented for `RwLockReadGuard<'_, Vec<Task>>`
```

Two fixes:

```rust
// GOOD: the guard is a temporary, dropped at the end of the statement.
let snapshot = store.read().map_err(|_| AppError::LockPoisoned)?.clone();
// ... any .await here is safe ...

// ALSO GOOD: use the async lock, whose guard IS Send (but every call must be .await-ed).
let snapshot = store.read().await.clone();
```

The bad pattern is usually an accidental one-liner:

```rust
// BAD: the guard lives until the end of the block, across the await
let guard = store.write().unwrap();
some_io().await;
guard.push(task);
```

For this feature no `.await` is needed inside the critical section at all, so
`std::sync::RwLock` with short scopes is both correct and faster. Keeping guards to one statement
is a habit worth forming now.

### 5.3 Deadlock and lock poisoning

Two classic `RwLock` misuses:

```rust
// DEADLOCK: `read` guard is still alive when `write` is requested.
// std::sync::RwLock is not re-entrant, so this hangs forever.
let tasks = store.read().unwrap();
store.write().unwrap().push(task);

// FIX: finish with the read guard (clone what you need), then take the write lock.
let count = store.read().unwrap().len();
store.write().unwrap().push(task);
```

A deadlocked request handler hangs a Tokio worker thread; with enough of them the server stops
responding entirely. The rule: **never call `read()` and `write()` on the same lock in one
statement-chain**, and prefer `let snapshot = ...clone();` to copy data out of the lock.

Poisoning is the other one: if any thread panics while holding the write guard, every later
`read()`/`write()` returns `Err(PoisonError)`. That is why the signatures are `Result`:

```rust
let snapshot = store.read().map_err(|_| AppError::LockPoisoned)?.clone();
```

Returning `500 {"error":"internal storage error"}` is correct here: the data may be in an
inconsistent state, and the project rule forbids `unwrap()` on request paths anyway. Keeping
panics out of the critical section makes this branch effectively unreachable.

### 5.4 Extractor rejections happen before your handler

`Json<CreateTask>` runs *before* `create_task` body. If the client's request is wrong, your code
never executes and axum answers on its own. Knowing the exact codes avoids hours of confusion:

| Client mistake | axum status | Rejection variant |
| --- | --- | --- |
| No `Content-Type: application/json` (very common with curl) | `415` | `MissingJsonContentType` |
| `{"title":` — not valid JSON at all | `400` | `JsonSyntaxError` |
| `{"title": 42}` — valid JSON, wrong type | `422` | `JsonDataError` |
| `{}` — required field missing | `422` | `JsonDataError` |
| Body over the 2 MB default limit | `413` | `BytesRejection` |

Consequences for the tests: assert `415` for the missing header separately from `400`/`422` for bad
bodies, and do not assume "bad request" always means `400`.

If you later want *all* errors to share your `{"error": "..."}` shape, replace the extractor with
one that captures the rejection:

```rust
async fn create_task(
    State(store): State<Store>,
    payload: Result<Json<CreateTask>, axum::extract::rejection::JsonRejection>,
) -> Result<..., AppError> { ... }
```

That is a deliberate trade (nicer client errors) and is not needed for this stage.

### 5.5 Extractor ordering: `Json` must be last

```rust
// OK
async fn create_task(State(store): State<Store>, Json(input): Json<CreateTask>) -> ...

// COMPILE ERROR: the body can only be consumed once, so it must be the final extractor
async fn create_task(Json(input): Json<CreateTask>, State(store): State<Store>) -> ...
```

Axum implements this rule for you — a body-consuming extractor that is not the last argument fails
to compile — but the error text ("the trait bound `...: FromRequestParts` is not satisfied") is not
obvious until you know the rule.

### 5.6 Trait-bound errors from missing derives

Two specific ones you will hit when adding the code:

```
the trait bound `CreateTask: Deserialize<'_>` is not satisfied
```
→ `CreateTask` is missing `#[derive(Deserialize)]`.

```
the trait bound `Store: Clone` is not satisfied
```
→ `State<T>`'s extractor requires `T: Clone + Send + Sync + 'static`; `Arc<_>` is `Clone`, but a
hand-written struct wrapping the store must also `#[derive(Clone)]`.

Also keep the derives on `Task` consistent: `Serialize` is needed to *send* a task, `Deserialize`
to accept it in tests. Both are already present.

### 5.7 State shared accidentally between tests

If the store were a `static`/`OnceLock` global, `created_task_appears_in_list` would pass and then
`list_tasks_returns_empty_json_array` would fail **depending on test order** — the worst kind of
flaky test. Building the store inside `app()` makes isolation automatic. Keep it that way; if a
future refactor needs a shared store across a test, construct it explicitly in that test.

### 5.8 Validation must not store half a task

Validate **before** taking the write lock and pushing:

```rust
let title = input.title.trim();
if title.is_empty() {
    return Err(AppError::InvalidTitle("title must not be empty".to_owned()));
}

let mut tasks = store.write().map_err(|_| AppError::LockPoisoned)?;
let task = Task { id, title: title.to_owned(), completed: input.completed };
tasks.push(task.clone());
Ok((StatusCode::CREATED, Json(task)))
```

Note `.trim()`, which also rejects `" "`. Trimming the stored value is a small behavioural choice
worth stating in the docs of `create_task` — clients that send `"  Buy milk  "` get the trimmed
version back, and the returned body is the source of truth.

### 5.9 Smaller pitfalls

- **Forgotten `.await`** (only if you choose `tokio::sync::RwLock`): `store.read()` returns a
  *future*, not a guard, so you get "no method `len` found" errors on a `RwLockReadFut`. Add
  `.await`.
- **`Arc` clones vs data clones**: `Arc::clone(&store)` shares, `store.read()?.clone()` copies.
  Both are needed here — and mixing them up silently makes tasks disappear.
- **`RwLock` writer starvation**: `std::sync::RwLock` does not guarantee that a waiting writer
  eventually wins, so a constant stream of `GET`s can delay a `POST`. Irrelevant at this scale;
  `tokio::sync::RwLock` is fair if it ever matters.
- **`u64` vs `usize` for `id`**: keep `u64` as it is now. Do not `as`-cast `usize` to `u64`
  silently; `u64::try_from(...)` is the honest conversion if you ever need it.
- **Naming drift**: the existing `Task` field is `completed`, not `done`. Keep the new
  `CreateTask` field spelled the same way, or clients will send the wrong key.

---

## 6. Appendices: full target code and commands

### 6.1 Suggested additions to `src/lib.rs`

```rust
use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};

/// Shared, in-memory task store. One lock guards the whole list.
pub type Store = Arc<RwLock<Vec<Task>>>;

/// Request body for `POST /tasks`.
///
/// The client supplies the content; the server assigns the `id`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreateTask {
    /// Human-readable description of the work item.
    pub title: String,
    /// Defaults to `false` when the field is omitted.
    #[serde(default)]
    pub completed: bool,
}

/// Error body shared by every failure response.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

/// Failures the handlers can report to a client.
#[derive(Debug)]
enum AppError {
    /// A thread panicked while holding the store lock.
    LockPoisoned,
    /// The submitted title was empty after trimming.
    InvalidTitle(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::LockPoisoned => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal storage error".to_owned(),
            ),
            AppError::InvalidTitle(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}

/// Builds the application router, with its own fresh store.
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/tasks", get(list_tasks).post(create_task))
        .with_state(Store::default())
}

/// `GET /tasks` — lists every stored task.
async fn list_tasks(State(store): State<Store>) -> Result<Json<Vec<Task>>, AppError> {
    // The guard is a temporary: it is dropped at the end of this statement,
    // so no lock is held while the response is serialised.
    let snapshot = store.read().map_err(|_| AppError::LockPoisoned)?.clone();

    Ok(Json(snapshot))
}

/// `POST /tasks` — creates a task and returns it with a server-assigned id.
async fn create_task(
    State(store): State<Store>,
    // The body-consuming extractor must be the last argument.
    Json(input): Json<CreateTask>,
) -> Result<(StatusCode, Json<Task>), AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::InvalidTitle("title must not be empty".to_owned()));
    }

    // One write lock covers both the id assignment and the push, so two
    // concurrent requests cannot pick the same id.
    let mut tasks = store.write().map_err(|_| AppError::LockPoisoned)?;
    let id = tasks.iter().map(|task| task.id).max().map_or(1, |max| max + 1);

    let task = Task {
        id,
        title: title.to_owned(),
        completed: input.completed,
    };
    tasks.push(task.clone());

    Ok((StatusCode::CREATED, Json(task)))
}
```

`health` and `list_tasks` keep their existing bodies; only `list_tasks` changes signature.
`main.rs` is untouched.

### 6.2 Commands to run once the code is written

```powershell
cargo fmt --check   # formatting
cargo check         # types, without linking
cargo test          # integration tests
cargo run           # then curl the endpoints from another shell
```

### 6.3 Deliberately out of scope

Persistence, `DELETE`/`PUT`, pagination, `tracing` logs, request-id headers, a `thiserror`-based
error type, and any auth. Each is a small, independent follow-up once this works.
