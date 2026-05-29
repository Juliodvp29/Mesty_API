use async_trait::async_trait;
use chrono::{DateTime, Utc};
use shared::error::DomainError;
use uuid::Uuid;

use super::entities::{Call, CallStatus, NewCall};

/// Trait abstracting persistence operations for calls.
/// Implemented by `PostgresCallRepository` in the infrastructure layer.
#[async_trait]
pub trait CallRepository: Send + Sync {
    /// Persists a new call record. Returns the created Call.
    async fn create(&self, new_call: NewCall) -> Result<Call, DomainError>;

    /// Retrieves a call by its ID. Returns `None` if not found.
    async fn find_by_id(&self, call_id: Uuid) -> Result<Option<Call>, DomainError>;

    /// Updates the status (and optionally started_at / ended_at) of a call.
    async fn update_status(
        &self,
        call_id: Uuid,
        status: CallStatus,
        started_at: Option<DateTime<Utc>>,
        ended_at: Option<DateTime<Utc>>,
    ) -> Result<Call, DomainError>;

    /// Returns true if the user is currently in an active call
    /// (status: initiated, ringing, or answered).
    async fn is_user_in_active_call(&self, user_id: Uuid) -> Result<bool, DomainError>;
}
