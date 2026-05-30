use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum DomainError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),
}

impl DomainError {
    pub fn from_sqlx(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => DomainError::NotFound("Resource not found".into()),
            sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
                Some("23505") => DomainError::AlreadyExists("Resource already exists".into()),
                Some("23503") => DomainError::Validation("Referenced resource not found".into()),
                Some("23514") => DomainError::Validation("Validation constraint failed".into()),
                _ => {
                    tracing::error!(db_error = %db_err, "Database error");
                    DomainError::Internal("Database operation failed".into())
                }
            },
            sqlx::Error::PoolTimedOut => {
                DomainError::Internal("Service temporarily unavailable".into())
            }
            _ => {
                tracing::error!(error = %e, "Unexpected database error");
                DomainError::Internal("An unexpected error occurred".into())
            }
        }
    }

    pub fn from_boxed_err(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        match e.downcast::<sqlx::Error>() {
            Ok(sqlx_err) => Self::from_sqlx(*sqlx_err),
            Err(other_err) => {
                tracing::error!(error = %other_err, "Unexpected database or service error");
                DomainError::Internal("An unexpected error occurred".into())
            }
        }
    }
}

pub type DomainResult<T> = Result<T, DomainError>;
