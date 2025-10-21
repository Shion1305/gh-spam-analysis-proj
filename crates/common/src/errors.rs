use std::fmt::Debug;

pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(#[from] config::ConfigError),
    #[error("database error: {0}")]
    Database(#[source] anyhow::Error),
    #[error("http error: {0}")]
    Http(#[source] anyhow::Error),
    #[error("not found: {0}")]
    NotFound(&'static str),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl AppError {
    pub fn db(err: impl Into<anyhow::Error>) -> Self {
        Self::Database(err.into())
    }

    pub fn http(err: impl Into<anyhow::Error>) -> Self {
        Self::Http(err.into())
    }
}
