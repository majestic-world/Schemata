use serde::Serialize;

pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Connection,
    Authentication,
    Sql,
    Cancelled,
    Io,
    Validation,
}

/// The single error type that crosses the command boundary to the UI.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    pub sql_state: Option<String>,
}

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), sql_state: None }
    }
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Validation, message)
    }
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Connection, message)
    }
    pub fn sql(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Sql, message)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Operation cancelled")
    }
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::io(format!("Invalid JSON: {e}"))
    }
}
