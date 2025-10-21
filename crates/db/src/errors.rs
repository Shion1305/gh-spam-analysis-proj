#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("query error: {0}")]
    Query(#[source] sqlx::Error),
    #[error("not found")]
    NotFound,
    #[error("configuration error: {0}")]
    Config(#[source] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;
