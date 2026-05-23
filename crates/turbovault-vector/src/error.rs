use thiserror::Error;

#[derive(Debug, Error)]
pub enum VectorError {
    #[error("vector index error: {0}")]
    Index(String),
    #[error("embedding error: {0}")]
    Embedding(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
}
