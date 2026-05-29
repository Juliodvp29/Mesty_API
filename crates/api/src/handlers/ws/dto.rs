use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsParams {
    pub ticket: String,
}

// ============================================================
//  CLIENT → SERVER MESSAGES
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsClientMessage {
    // ---- Chat ----
    #[serde(rename = "typing_start")]
    TypingStart { chat_id: Uuid },
    #[serde(rename = "typing_stop")]
    TypingStop { chat_id: Uuid },
    #[serde(rename = "sync_request")]
    SyncRequest {
        since: Option<chrono::DateTime<chrono::Utc>>,
    },

    // ---- WebRTC Signaling ----
    /// Caller initiates a call: sends SDP offer to the server so that it
    /// can be relayed to the receiver via their connected WebSocket.
    #[serde(rename = "call:initiate")]
    CallInitiate {
        receiver_id: Uuid,
        /// "audio" | "video"
        call_type: String,
        /// SDP Offer from the caller (opaque JSON passthrough — not persisted).
        offer: serde_json::Value,
    },

    /// Receiver accepts and returns its SDP answer.
    #[serde(rename = "call:accept")]
    CallAccept {
        call_id: Uuid,
        /// SDP Answer from the receiver (opaque JSON passthrough — not persisted).
        answer: serde_json::Value,
    },

    /// Receiver declines the call.
    #[serde(rename = "call:reject")]
    CallReject {
        call_id: Uuid,
        /// Optional reason: "rejected" | "busy"
        reason: Option<String>,
    },

    /// Either party sends an ICE candidate to be relayed to the peer.
    #[serde(rename = "call:ice-candidate")]
    CallIceCandidate {
        call_id: Uuid,
        /// The peer that should receive this candidate.
        receiver_id: Uuid,
        /// ICE candidate object (opaque JSON passthrough — not persisted).
        candidate: serde_json::Value,
    },

    /// Either party hangs up.
    #[serde(rename = "call:hangup")]
    CallHangup { call_id: Uuid },
}

// ============================================================
//  SERVER → CLIENT MESSAGES
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsServerMessage {
    // ---- Chat ----
    #[serde(rename = "new_message")]
    NewMessage(NewMessagePayload),
    #[serde(rename = "message_edited")]
    MessageEdited(MessageEditedPayload),
    #[serde(rename = "message_deleted")]
    MessageDeleted(MessageDeletedPayload),
    #[serde(rename = "reaction_added")]
    ReactionAdded(ReactionPayload),
    #[serde(rename = "reaction_removed")]
    ReactionRemoved(ReactionPayload),
    #[serde(rename = "messages_read")]
    MessagesRead(MessagesReadPayload),
    #[serde(rename = "user_online")]
    UserOnline(UserPresencePayload),
    #[serde(rename = "user_offline")]
    UserOffline(UserPresencePayload),
    #[serde(rename = "typing_start")]
    TypingStart(TypingPayload),
    #[serde(rename = "typing_stop")]
    TypingStop(TypingPayload),
    #[serde(rename = "key_changed")]
    KeyChanged(KeyChangedPayload),

    // ---- WebRTC Signaling ----
    /// Sent to the receiver when someone initiates a call.
    #[serde(rename = "call:incoming")]
    CallIncoming(CallIncomingPayload),

    /// Sent to the caller when the receiver accepts.
    #[serde(rename = "call:accepted")]
    CallAccepted(CallAcceptedPayload),

    /// Sent to the caller when the receiver declines.
    #[serde(rename = "call:rejected")]
    CallRejected(CallRejectedPayload),

    /// Relayed ICE candidate between peers.
    #[serde(rename = "call:ice-candidate")]
    CallIceCandidate(CallIceCandidatePayload),

    /// Sent to both parties when the call is terminated.
    #[serde(rename = "call:ended")]
    CallEnded(CallEndedPayload),
}

// ============================================================
//  PAYLOAD STRUCTS - Chat
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessagePayload {
    pub chat_id: Uuid,
    pub message: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEditedPayload {
    pub chat_id: Uuid,
    pub message_id: Uuid,
    pub content_encrypted: String,
    pub content_iv: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeletedPayload {
    pub chat_id: Uuid,
    pub message_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionPayload {
    pub chat_id: Uuid,
    pub message_id: Uuid,
    pub reaction: String,
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesReadPayload {
    pub chat_id: Uuid,
    pub user_id: Uuid,
    pub up_to: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPresencePayload {
    pub user_id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingPayload {
    pub chat_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyChangedPayload {
    pub user_id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ============================================================
//  PAYLOAD STRUCTS - WebRTC Calls
// ============================================================

/// Delivered to the receiver side to alert them of an incoming call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallIncomingPayload {
    pub call_id: Uuid,
    pub caller_id: Uuid,
    /// "audio" | "video"
    pub call_type: String,
    /// SDP Offer from the caller (relayed as-is from the signaling event).
    pub offer: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Delivered to the caller once the receiver has accepted the call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallAcceptedPayload {
    pub call_id: Uuid,
    pub receiver_id: Uuid,
    /// SDP Answer from the receiver (relayed as-is).
    pub answer: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Delivered to the caller when the receiver declines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRejectedPayload {
    pub call_id: Uuid,
    pub receiver_id: Uuid,
    /// "rejected" | "busy"
    pub reason: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Relays an ICE candidate to the remote peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallIceCandidatePayload {
    pub call_id: Uuid,
    /// The user who sent this candidate (so the receiver knows who it came from).
    pub sender_id: Uuid,
    /// ICE candidate object (relayed as-is).
    pub candidate: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Delivered to both participants when the call terminates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEndedPayload {
    pub call_id: Uuid,
    /// The user who triggered the termination.
    pub ended_by: Uuid,
    /// Final status: "ended" | "missed" | "rejected" | "busy"
    pub status: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
