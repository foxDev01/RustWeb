//! SQLite storage layer.
//!
//! The application shares a single `rusqlite::Connection` behind a
//! `std::sync::Mutex`. `rusqlite` is a blocking library, so every statement runs
//! inside [`tokio::task::spawn_blocking`]: the database may take a few
//! milliseconds, and the async worker threads must stay free to serve requests.
//!
//! All SQL is parameterised (`?1`, `?2`, ...). User input is never formatted
//! into a query string.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::models::{Task, ValidPatch, ValidTask};

/// Schema applied every time the database is opened.
///
/// Both statements are idempotent. A larger application would use versioned
/// migrations; for a single-table MVP `IF NOT EXISTS` keeps this honest and
/// short. `AUTOINCREMENT` guarantees that a task id is never reused, even after
/// the highest-numbered task is deleted.
const SCHEMA: &str = "\
PRAGMA busy_timeout = 5000;
CREATE TABLE IF NOT EXISTS tasks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT    NOT NULL,
    description TEXT,
    completed   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS tasks_completed_idx ON tasks (completed);
";

/// Errors raised by the storage layer.
#[derive(Debug)]
pub enum DbError {
    /// SQLite reported a failure.
    Sqlite(rusqlite::Error),
    /// The blocking task that ran a query panicked or was cancelled.
    Task(tokio::task::JoinError),
    /// Another thread panicked while holding the connection lock, so the
    /// connection may be in an inconsistent state.
    Poisoned,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::Task(error) => write!(formatter, "blocking database task failed: {error}"),
            Self::Poisoned => formatter.write_str("database connection was poisoned by a panic"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Task(error) => Some(error),
            Self::Poisoned => None,
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<tokio::task::JoinError> for DbError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::Task(error)
    }
}

/// Handle to the SQLite database.
///
/// Cheap to clone: every clone points at the same connection, which is what the
/// axum state needs (`Clone + Send + Sync + 'static`).
#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    /// Opens (creating it if necessary) the database file at `path` and applies
    /// the schema.
    ///
    /// The path `:memory:` opens a private in-memory database, which is handy for
    /// experimenting without leaving a file behind.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    /// Opens a fresh in-memory database. Used by the tests so each one is
    /// completely isolated from the others.
    pub fn open_in_memory() -> Result<Self, DbError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    /// Applies the schema and wraps the connection in the shared handle.
    fn from_connection(connection: Connection) -> Result<Self, DbError> {
        connection.execute_batch(SCHEMA)?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Inserts a validated task and returns the stored row, including the id
    /// assigned by SQLite.
    pub async fn create(&self, task: ValidTask) -> Result<Task, DbError> {
        with_connection(&self.connection, move |connection| {
            // `RETURNING` (SQLite 3.35+) gives back the generated id in the same
            // statement, so no second query is needed.
            connection
                .query_row(
                    "INSERT INTO tasks (title, description, completed)
                     VALUES (?1, ?2, ?3)
                     RETURNING id, title, description, completed",
                    params![task.title, task.description, task.completed],
                    task_from_row,
                )
                .map_err(DbError::from)
        })
        .await
    }

    /// Lists tasks, optionally restricted to active (`Some(false)`) or completed
    /// (`Some(true)`) ones. `None` returns everything.
    ///
    /// The filter is a bound parameter rather than concatenated SQL: when it is
    /// `NULL` the `(?1 IS NULL OR ...)` test accepts every row.
    pub async fn list(&self, completed: Option<bool>) -> Result<Vec<Task>, DbError> {
        with_connection(&self.connection, move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, title, description, completed
                   FROM tasks
                  WHERE (?1 IS NULL OR completed = ?1)
                  ORDER BY id ASC",
            )?;

            let rows = statement.query_map(params![completed], task_from_row)?;
            let tasks: Vec<Task> = rows.collect::<rusqlite::Result<Vec<Task>>>()?;

            Ok(tasks)
        })
        .await
    }

    /// Applies a partial update and returns the stored task, or `None` when no
    /// task has that id.
    pub async fn update(&self, id: i64, patch: ValidPatch) -> Result<Option<Task>, DbError> {
        with_connection(&self.connection, move |connection| {
            // Read, modify and write while holding the lock exactly once. The
            // helpers below take `&Connection` instead of locking again, because
            // `Mutex` is not re-entrant and would deadlock.
            let Some(current) = load(connection, id)? else {
                return Ok(None);
            };

            let title = patch.title.unwrap_or(current.title);
            let description = match patch.description {
                // `Some(Some(text))` sets it, `Some(None)` clears it...
                Some(description) => description,
                // ...and `None` keeps whatever is stored.
                None => current.description,
            };
            let completed = patch.completed.unwrap_or(current.completed);

            connection.execute(
                "UPDATE tasks SET title = ?1, description = ?2, completed = ?3 WHERE id = ?4",
                params![title, description, completed, id],
            )?;

            load(connection, id)
        })
        .await
    }

    /// Deletes a task. Returns `false` when no task has that id, so the caller
    /// can answer `404` instead of pretending the delete worked.
    pub async fn delete(&self, id: i64) -> Result<bool, DbError> {
        with_connection(&self.connection, move |connection| {
            let affected = connection.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;

            Ok(affected > 0)
        })
        .await
    }

    /// Reads a single task.
    pub async fn get(&self, id: i64) -> Result<Option<Task>, DbError> {
        with_connection(&self.connection, move |connection| load(connection, id)).await
    }
}

/// Runs a closure that uses the shared connection inside a blocking task.
///
/// Takes `&Arc<...>` and clones the handle, so the returned future owns
/// everything it needs and the caller can keep using `self`.
async fn with_connection<T, F>(
    connection: &Arc<Mutex<Connection>>,
    operation: F,
) -> Result<T, DbError>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, DbError> + Send + 'static,
{
    let connection = Arc::clone(connection);

    tokio::task::spawn_blocking(move || {
        let guard = connection.lock().map_err(|_| DbError::Poisoned)?;
        operation(&guard)
    })
    .await?
}

/// Reads one task through an already-locked connection.
fn load(connection: &Connection, id: i64) -> Result<Option<Task>, DbError> {
    connection
        .query_row(
            "SELECT id, title, description, completed FROM tasks WHERE id = ?1",
            params![id],
            task_from_row,
        )
        // `optional` turns "no rows" into `None` instead of an error.
        .optional()
        .map_err(DbError::from)
}

/// Maps a result row onto [`Task`]. Column names are used so the mapping keeps
/// working if the `SELECT` list is reordered.
fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get("id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        completed: row.get("completed")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(title: &str, description: Option<&str>, completed: bool) -> ValidTask {
        ValidTask {
            title: title.to_owned(),
            description: description.map(str::to_owned),
            completed,
        }
    }

    fn patch(
        title: Option<&str>,
        description: Option<Option<&str>>,
        completed: Option<bool>,
    ) -> ValidPatch {
        ValidPatch {
            title: title.map(str::to_owned),
            description: description.map(|text| text.map(str::to_owned)),
            completed,
        }
    }

    /// Fails the test if a query unexpectedly found no row. Uses `panic!` rather
    /// than `unwrap` so the codebase contains no `unwrap` at all.
    fn unwrap_task(task: Option<Task>) -> Task {
        match task {
            Some(task) => task,
            None => panic!("expected the task to exist"),
        }
    }

    #[tokio::test]
    async fn create_assigns_sequential_ids_and_lists_in_order() -> Result<(), DbError> {
        let database = Database::open_in_memory()?;

        let first = database.create(valid("first", None, false)).await?;
        let second = database
            .create(valid("second", Some("details"), true))
            .await?;

        assert_eq!(first.id, 1);
        assert_eq!(second.id, 2);
        assert_eq!(second.description.as_deref(), Some("details"));
        assert_eq!(database.list(None).await?, vec![first, second]);
        Ok(())
    }

    #[tokio::test]
    async fn list_filters_by_completion() -> Result<(), DbError> {
        let database = Database::open_in_memory()?;
        database.create(valid("active", None, false)).await?;
        database.create(valid("done", None, true)).await?;

        let titles = |tasks: Vec<Task>| {
            tasks
                .into_iter()
                .map(|task| task.title)
                .collect::<Vec<String>>()
        };

        assert_eq!(titles(database.list(None).await?).len(), 2);
        assert_eq!(titles(database.list(Some(false)).await?), vec!["active"]);
        assert_eq!(titles(database.list(Some(true)).await?), vec!["done"]);
        Ok(())
    }

    #[tokio::test]
    async fn update_changes_only_the_fields_it_is_given() -> Result<(), DbError> {
        let database = Database::open_in_memory()?;
        let task = database.create(valid("title", Some("body"), false)).await?;

        let renamed = unwrap_task(
            database
                .update(task.id, patch(Some("renamed"), None, None))
                .await?,
        );

        assert_eq!(renamed.title, "renamed");
        assert_eq!(renamed.description.as_deref(), Some("body"), "unchanged");
        assert!(!renamed.completed, "unchanged");

        let toggled = unwrap_task(
            database
                .update(task.id, patch(None, None, Some(true)))
                .await?,
        );
        assert_eq!(toggled.title, "renamed");
        assert!(toggled.completed);

        let cleared = unwrap_task(
            database
                .update(task.id, patch(None, Some(None), None))
                .await?,
        );
        assert_eq!(cleared.description, None);
        Ok(())
    }

    #[tokio::test]
    async fn update_and_delete_report_missing_rows() -> Result<(), DbError> {
        let database = Database::open_in_memory()?;

        assert_eq!(
            database.update(404, patch(Some("x"), None, None)).await?,
            None
        );
        assert!(database.get(404).await?.is_none());
        assert!(!database.delete(404).await?);
        Ok(())
    }

    #[tokio::test]
    async fn delete_removes_the_task() -> Result<(), DbError> {
        let database = Database::open_in_memory()?;
        let task = database.create(valid("temporary", None, false)).await?;

        assert!(database.delete(task.id).await?);
        assert!(database.list(None).await?.is_empty());
        Ok(())
    }
}
