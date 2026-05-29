use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::call::entities::{Call, CallStatus, CallType, NewCall};
use domain::call::repository::CallRepository;
use shared::error::DomainError;
use sqlx::PgPool;
use uuid::Uuid;

/// Postgres-backed implementation of `CallRepository`.
///
/// NOTE: We use `sqlx::query_as` (not the `query!` macro) for queries that
/// involve custom Postgres ENUMs (`call_type`, `call_status`) because sqlx's
/// compile-time macros cannot automatically map custom ENUM types to Rust
/// primitives without deriving `PgType`.  We cast the ENUM columns to `TEXT`
/// in the SELECT list and bind parameters as `&str`, keeping full type safety
/// at the Rust layer through our own `from_db_str` helpers.
pub struct PostgresCallRepository {
    pool: PgPool,
}

impl PostgresCallRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Raw row returned by queries that SELECT with `::text` casts on ENUMs.
#[derive(sqlx::FromRow)]
struct CallRow {
    id: Uuid,
    caller_id: Uuid,
    receiver_id: Uuid,
    call_type: Option<String>,
    status: Option<String>,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<CallRow> for Call {
    type Error = DomainError;

    fn try_from(row: CallRow) -> Result<Self, Self::Error> {
        let call_type =
            CallType::from_db_str(row.call_type.as_deref().unwrap_or("")).ok_or_else(|| {
                DomainError::Internal(format!("Unknown call_type: {:?}", row.call_type))
            })?;
        let status =
            CallStatus::from_db_str(row.status.as_deref().unwrap_or("")).ok_or_else(|| {
                DomainError::Internal(format!("Unknown call_status: {:?}", row.status))
            })?;

        Ok(Call {
            id: row.id,
            caller_id: row.caller_id,
            receiver_id: row.receiver_id,
            call_type,
            status,
            started_at: row.started_at,
            ended_at: row.ended_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const SELECT_FIELDS: &str = r#"
    id,
    caller_id,
    receiver_id,
    type::text   AS call_type,
    status::text AS status,
    started_at,
    ended_at,
    created_at,
    updated_at
"#;

#[async_trait]
impl CallRepository for PostgresCallRepository {
    async fn create(&self, new_call: NewCall) -> Result<Call, DomainError> {
        let call_type_str = new_call.call_type.as_db_str();

        let row: CallRow = sqlx::query_as(&format!(
            r#"
            INSERT INTO calls (caller_id, receiver_id, type, status)
            VALUES ($1, $2, $3::call_type, 'initiated'::call_status)
            RETURNING {}
            "#,
            SELECT_FIELDS
        ))
        .bind(new_call.caller_id)
        .bind(new_call.receiver_id)
        .bind(call_type_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DomainError::Internal(format!("DB error creating call: {}", e)))?;

        row.try_into()
    }

    async fn find_by_id(&self, call_id: Uuid) -> Result<Option<Call>, DomainError> {
        let row: Option<CallRow> = sqlx::query_as(&format!(
            r#"
            SELECT {} FROM calls WHERE id = $1
            "#,
            SELECT_FIELDS
        ))
        .bind(call_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DomainError::Internal(format!("DB error fetching call: {}", e)))?;

        row.map(Call::try_from).transpose()
    }

    async fn update_status(
        &self,
        call_id: Uuid,
        status: CallStatus,
        started_at: Option<DateTime<Utc>>,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<Call, DomainError> {
        let status_str = status.as_db_str();

        let row: CallRow = sqlx::query_as(&format!(
            r#"
            UPDATE calls
            SET
                status     = $2::call_status,
                started_at = COALESCE($3, started_at),
                ended_at   = $4
            WHERE id = $1
            RETURNING {}
            "#,
            SELECT_FIELDS
        ))
        .bind(call_id)
        .bind(status_str)
        .bind(started_at)
        .bind(ended_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DomainError::Internal(format!("DB error updating call status: {}", e)))?;

        row.try_into()
    }

    async fn is_user_in_active_call(&self, user_id: Uuid) -> Result<bool, DomainError> {
        let (exists,): (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM calls
                WHERE (caller_id = $1 OR receiver_id = $1)
                  AND status::text = ANY(ARRAY['initiated', 'ringing', 'answered'])
            )
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DomainError::Internal(format!("DB error checking active call: {}", e)))?;

        Ok(exists)
    }
}
