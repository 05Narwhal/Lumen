use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("no library is open")]
    NoLibrary,
    #[error("migration failed: {0}")]
    Migration(String),
    /// A request the data refuses (an empty name, a built-in preset changed).
    #[error("invalid: {0}")]
    Invalid(String),
    /// The library was last opened by a newer Lumen (Round 17). It is left as it is.
    #[error("the library is at schema {library}, this build knows {app}")]
    NewerSchema { library: u32, app: u32 },
    /// The copy before a migration could not be made (Round 17), so the migration did not run.
    #[error("safety copy failed: {0}")]
    SafetyCopy(String),
}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::Io(_) => "io",
            CoreError::Db(_) => "db",
            CoreError::Json(_) => "json",
            CoreError::NotFound(_) => "not_found",
            CoreError::AlreadyExists(_) => "already_exists",
            CoreError::InvalidPath(_) => "invalid_path",
            CoreError::NoLibrary => "no_library",
            CoreError::Migration(_) => "migration_failed",
            CoreError::Invalid(_) => "invalid",
            CoreError::NewerSchema { .. } => "library_too_new",
            CoreError::SafetyCopy(_) => "safety_copy_failed",
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_stable_code() {
        assert_eq!(CoreError::NoLibrary.code(), "no_library");
        assert_eq!(CoreError::NotFound("x".into()).code(), "not_found");
        assert_eq!(CoreError::Migration("x".into()).code(), "migration_failed");
        assert_eq!(CoreError::Invalid("x".into()).code(), "invalid");
        let io: CoreError = std::io::Error::other("boom").into();
        assert_eq!(io.code(), "io");
    }
}
