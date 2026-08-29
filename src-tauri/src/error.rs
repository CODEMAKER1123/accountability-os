use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("AI provider error: {0}")]
    Ai(String),
    #[error("secret storage error: {0}")]
    Secrets(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::Invalid(msg.into())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(format!("serialization: {e}"))
    }
}

/// Tauri command errors cross IPC as a structured payload the UI can show.
#[derive(Serialize)]
struct ErrorPayload {
    code: &'static str,
    message: String,
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let code = match self {
            AppError::Db(_) => "db",
            AppError::Invalid(_) => "invalid",
            AppError::NotFound(_) => "not_found",
            AppError::Ai(_) => "ai",
            AppError::Secrets(_) => "secrets",
            AppError::Io(_) => "io",
            AppError::Internal(_) => "internal",
        };
        ErrorPayload {
            code,
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

pub type AppResult<T> = Result<T, AppError>;
