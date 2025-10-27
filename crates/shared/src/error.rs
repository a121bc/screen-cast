use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("Media Foundation support unavailable: {0}")]
    MediaFoundation(&'static str),
    #[error("Win32 API error: {0}")]
    Win32(&'static str),
}

pub type AppResult<T> = Result<T, AppError>;
