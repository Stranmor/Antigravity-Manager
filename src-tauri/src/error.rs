use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "desktop")]
    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Account error: {0}")]
    Account(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

// Implement Serialize to allow usage as Tauri command return value
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

// Result type alias for convenience
pub type AppResult<T> = Result<T, AppError>;
