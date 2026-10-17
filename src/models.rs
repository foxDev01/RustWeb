//! Data models shared by the HTTP layer and the storage layer.
//!
//! Every request body is validated here before it reaches SQLite: trimming
//! whitespace, rejecting empty or oversized values and normalising "no
//! description" to `None`.

use serde::{Deserialize, Serialize};

/// Maximum accepted length of a task title, counted in characters.
pub const MAX_TITLE_CHARS: usize = 200;

/// Maximum accepted length of a task description, counted in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;

/// A task as stored in the database and returned to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Server-assigned identifier (SQLite `INTEGER`, so a signed 64-bit value).
    pub id: i64,
    /// Human-readable description of the work item.
    pub title: String,
    /// Optional extra details. `None` means "no description".
    pub description: Option<String>,
    /// Whether the task has been finished.
    pub completed: bool,
}

/// Body returned by `GET /health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    /// Always `"ok"` while the process is able to serve requests.
    pub status: String,
}

/// Request body of `POST /tasks`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewTask {
    /// Required. Trimmed before validation; whitespace-only is rejected.
    pub title: String,
    /// Optional. An empty string is treated as "no description".
    #[serde(default)]
    pub description: Option<String>,
    /// Optional; defaults to `false`.
    #[serde(default)]
    pub completed: bool,
}

/// Request body of `PATCH /tasks/{id}`.
///
/// Every field is optional so a client can send only what changed:
///
/// * `None` or `null` — leave the stored value untouched;
/// * `Some("")` for `description` — clear the description;
/// * an empty `title` is rejected rather than clearing it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct TaskPatch {
    /// New title, if the client wants to change it.
    #[serde(default)]
    pub title: Option<String>,
    /// New description. An empty string clears it.
    #[serde(default)]
    pub description: Option<String>,
    /// New completion flag.
    #[serde(default)]
    pub completed: Option<bool>,
}

/// A validated, normalised task that is ready to be written to storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTask {
    /// Trimmed, non-empty title.
    pub title: String,
    /// Trimmed description, or `None` when there was no meaningful text.
    pub description: Option<String>,
    /// Requested completion flag.
    pub completed: bool,
}

/// A validated update.
///
/// The nested option for `description` distinguishes three states, which is the
/// usual Rust encoding for "absent / clear / set":
///
/// | Value | Meaning |
/// | --- | --- |
/// | `None` | leave the stored description unchanged |
/// | `Some(None)` | clear the description |
/// | `Some(Some(text))` | replace it with `text` |
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidPatch {
    /// Replacement title, if any.
    pub title: Option<String>,
    /// Replacement description, if any.
    pub description: Option<Option<String>>,
    /// Replacement completion flag, if any.
    pub completed: Option<bool>,
}

/// Why a request body or query string was rejected.
///
/// Handlers turn this into `422 Unprocessable Entity` with a readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    /// Builds an error with a client-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message that is safe to show to a client.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

impl NewTask {
    /// Validates a new task and normalises the text it contains.
    pub fn validate(self) -> Result<ValidTask, ValidationError> {
        let title = self.title.trim().to_owned();
        validate_title(&title)?;

        let description = match self.description {
            Some(text) => validate_description(text.trim())?,
            None => None,
        };

        Ok(ValidTask {
            title,
            description,
            completed: self.completed,
        })
    }
}

impl TaskPatch {
    /// Validates the fields that are present and leaves the rest untouched.
    pub fn validate(self) -> Result<ValidPatch, ValidationError> {
        let title = match self.title {
            Some(title) => {
                let trimmed = title.trim().to_owned();
                validate_title(&trimmed)?;
                Some(trimmed)
            }
            None => None,
        };

        let description = match self.description {
            Some(text) => Some(validate_description(text.trim())?),
            None => None,
        };

        Ok(ValidPatch {
            title,
            description,
            completed: self.completed,
        })
    }
}

/// Rejects empty and oversized titles.
fn validate_title(title: &str) -> Result<(), ValidationError> {
    if title.is_empty() {
        return Err(ValidationError::new("title must not be empty"));
    }

    // `chars()` rather than `len()`: UTF-8 text is measured in characters, so a
    // title of 200 emoji is rejected while 200 ASCII letters are accepted.
    let length = title.chars().count();
    if length > MAX_TITLE_CHARS {
        return Err(ValidationError::new(format!(
            "title must be at most {MAX_TITLE_CHARS} characters, got {length}"
        )));
    }

    Ok(())
}

/// Normalises a description: empty text becomes "no description".
fn validate_description(description: &str) -> Result<Option<String>, ValidationError> {
    if description.is_empty() {
        return Ok(None);
    }

    let length = description.chars().count();
    if length > MAX_DESCRIPTION_CHARS {
        return Err(ValidationError::new(format!(
            "description must be at most {MAX_DESCRIPTION_CHARS} characters, got {length}"
        )));
    }

    Ok(Some(description.to_owned()))
}

/// The `?filter=` values accepted by `GET /tasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskFilter {
    /// Every task.
    #[default]
    All,
    /// Tasks that are not completed yet.
    Active,
    /// Tasks that are completed.
    Completed,
}

impl TaskFilter {
    /// The value bound to the `completed` column: `None` means "any value".
    pub fn completed_flag(self) -> Option<bool> {
        match self {
            Self::All => None,
            Self::Active => Some(false),
            Self::Completed => Some(true),
        }
    }
}

/// Query string of `GET /tasks`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ListQuery {
    /// `all` (the default), `active` or `completed`.
    #[serde(default)]
    pub filter: Option<String>,
}

impl ListQuery {
    /// Turns the raw query value into a filter, rejecting unknown values with a
    /// message that lists the accepted ones.
    pub fn filter(&self) -> Result<TaskFilter, ValidationError> {
        match self.filter.as_deref().map(str::trim) {
            None | Some("") | Some("all") => Ok(TaskFilter::All),
            Some("active") => Ok(TaskFilter::Active),
            Some("completed") => Ok(TaskFilter::Completed),
            Some(other) => Err(ValidationError::new(format!(
                "unknown filter `{other}`; expected `all`, `active` or `completed`"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_task(title: &str, description: Option<&str>) -> NewTask {
        NewTask {
            title: title.to_owned(),
            description: description.map(str::to_owned),
            completed: false,
        }
    }

    #[test]
    fn validation_trims_surrounding_whitespace() -> Result<(), ValidationError> {
        let valid = new_task("  buy milk  ", Some("  two litres  ")).validate()?;

        assert_eq!(valid.title, "buy milk");
        assert_eq!(valid.description.as_deref(), Some("two litres"));
        Ok(())
    }

    #[test]
    fn validation_rejects_empty_or_whitespace_titles() {
        assert!(new_task("", None).validate().is_err());
        assert!(new_task("   ", None).validate().is_err());
    }

    #[test]
    fn validation_rejects_oversized_fields() {
        assert!(
            new_task(&"a".repeat(MAX_TITLE_CHARS + 1), None)
                .validate()
                .is_err()
        );
        assert!(
            new_task("ok", Some(&"b".repeat(MAX_DESCRIPTION_CHARS + 1)))
                .validate()
                .is_err()
        );

        // The limit itself is inclusive, and is measured in characters.
        assert!(
            new_task(&"a".repeat(MAX_TITLE_CHARS), None)
                .validate()
                .is_ok()
        );
        assert!(
            new_task("🚀".repeat(MAX_TITLE_CHARS).as_str(), None)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn validation_treats_blank_description_as_absent() -> Result<(), ValidationError> {
        let valid = new_task("title", Some("   ")).validate()?;

        assert_eq!(valid.description, None);
        Ok(())
    }

    #[test]
    fn patch_distinguishes_unchanged_cleared_and_set() -> Result<(), ValidationError> {
        let unchanged = TaskPatch::default().validate()?;
        assert_eq!(unchanged.description, None);

        let cleared = TaskPatch {
            description: Some(String::new()),
            ..TaskPatch::default()
        }
        .validate()?;
        assert_eq!(cleared.description, Some(None));

        let set = TaskPatch {
            description: Some("  details  ".to_owned()),
            ..TaskPatch::default()
        }
        .validate()?;
        assert_eq!(set.description, Some(Some("details".to_owned())));
        Ok(())
    }

    #[test]
    fn patch_rejects_blank_title() {
        let patch = TaskPatch {
            title: Some("   ".to_owned()),
            ..TaskPatch::default()
        };

        assert!(patch.validate().is_err());
    }

    #[test]
    fn list_query_maps_known_filters_and_rejects_unknown_ones() {
        let query = |value: Option<&str>| ListQuery {
            filter: value.map(str::to_owned),
        };

        assert_eq!(query(None).filter(), Ok(TaskFilter::All));
        assert_eq!(query(Some("")).filter(), Ok(TaskFilter::All));
        assert_eq!(query(Some("all")).filter(), Ok(TaskFilter::All));
        assert_eq!(query(Some("active")).filter(), Ok(TaskFilter::Active));
        assert_eq!(query(Some("completed")).filter(), Ok(TaskFilter::Completed));

        // Unknown values are rejected with a message that lists what is allowed.
        match query(Some("everything")).filter() {
            Ok(filter) => panic!("`everything` should be rejected, got {filter:?}"),
            Err(error) => assert!(
                error.message().contains("unknown filter"),
                "unexpected message: {}",
                error.message()
            ),
        }
    }

    #[test]
    fn filters_map_to_sql_flags() {
        assert_eq!(TaskFilter::All.completed_flag(), None);
        assert_eq!(TaskFilter::Active.completed_flag(), Some(false));
        assert_eq!(TaskFilter::Completed.completed_flag(), Some(true));
    }
}
