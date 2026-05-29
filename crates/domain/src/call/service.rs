use chrono::Utc;
use shared::error::DomainError;
use std::sync::Arc;
use uuid::Uuid;

use super::entities::{Call, CallStatus, CallType, NewCall};
use super::repository::CallRepository;

/// Orchestrates business rules for call lifecycle.
pub struct CallService {
    call_repo: Arc<dyn CallRepository>,
}

impl CallService {
    pub fn new(call_repo: Arc<dyn CallRepository>) -> Self {
        Self { call_repo }
    }

    /// Creates a call record and verifies that neither party is already in an
    /// active call (returns `DomainError::Conflict` if busy).
    pub async fn initiate_call(
        &self,
        caller_id: Uuid,
        receiver_id: Uuid,
        call_type: CallType,
    ) -> Result<Call, DomainError> {
        if caller_id == receiver_id {
            return Err(DomainError::BadRequest("Cannot call yourself".to_string()));
        }

        // Detect if receiver is already busy
        if self.call_repo.is_user_in_active_call(receiver_id).await? {
            return Err(DomainError::Conflict(
                "Receiver is currently busy in another call".to_string(),
            ));
        }

        // Detect if caller is already in an active call
        if self.call_repo.is_user_in_active_call(caller_id).await? {
            return Err(DomainError::Conflict(
                "You are already in an active call".to_string(),
            ));
        }

        let call = self
            .call_repo
            .create(NewCall {
                caller_id,
                receiver_id,
                call_type,
            })
            .await?;

        Ok(call)
    }

    /// Transitions the call to `ringing` status, confirming the receiver was
    /// notified (WS event delivered / push sent).
    pub async fn mark_ringing(&self, call_id: Uuid) -> Result<Call, DomainError> {
        self.call_repo
            .update_status(call_id, CallStatus::Ringing, None, None)
            .await
    }

    /// Receiver accepts — transition to `answered` and record `started_at`.
    pub async fn accept_call(&self, call_id: Uuid, user_id: Uuid) -> Result<Call, DomainError> {
        let call = self
            .call_repo
            .find_by_id(call_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("Call not found".to_string()))?;

        if call.receiver_id != user_id {
            return Err(DomainError::Unauthorized(
                "Only the receiver can accept a call".to_string(),
            ));
        }

        if !matches!(call.status, CallStatus::Initiated | CallStatus::Ringing) {
            return Err(DomainError::BadRequest(format!(
                "Cannot accept a call in status: {:?}",
                call.status
            )));
        }

        self.call_repo
            .update_status(call_id, CallStatus::Answered, Some(Utc::now()), None)
            .await
    }

    /// Either party can reject/hang up a call.
    /// - If still `initiated`/`ringing` and rejected by receiver -> `Rejected`
    /// - If receiver is `busy` -> `Busy`
    /// - If the caller hangs up before answered -> `Missed`
    /// - After `answered`, both parties can `End` it -> `Ended`
    pub async fn end_call(
        &self,
        call_id: Uuid,
        user_id: Uuid,
        requested_status: CallStatus,
    ) -> Result<Call, DomainError> {
        let call = self
            .call_repo
            .find_by_id(call_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("Call not found".to_string()))?;

        // Only participants can end a call
        if call.caller_id != user_id && call.receiver_id != user_id {
            return Err(DomainError::Unauthorized(
                "You are not a participant of this call".to_string(),
            ));
        }

        // Only allow terminal transitions from active states
        if !call.status.is_active() {
            return Err(DomainError::BadRequest(format!(
                "Call is already in a terminal state: {:?}",
                call.status
            )));
        }

        let terminal = match requested_status {
            CallStatus::Ended | CallStatus::Missed | CallStatus::Rejected | CallStatus::Busy => {
                requested_status
            }
            _ => {
                return Err(DomainError::BadRequest(
                    "Invalid terminal status requested".to_string(),
                ));
            }
        };

        self.call_repo
            .update_status(call_id, terminal, call.started_at, Some(Utc::now()))
            .await
    }
}
