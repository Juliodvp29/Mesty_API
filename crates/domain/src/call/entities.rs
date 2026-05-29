use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The type of a call: voice-only or with video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Audio,
    Video,
}

impl CallType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "audio" => Some(Self::Audio),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

/// The lifecycle status of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// Caller sent the initiate event but receiver has not been notified yet.
    Initiated,
    /// Receiver has been notified (WS / Push delivered), waiting for answer.
    Ringing,
    /// Receiver accepted; WebRTC negotiation in progress.
    Answered,
    /// Call terminated normally by either party.
    Ended,
    /// Receiver never answered (caller hung up or timeout).
    Missed,
    /// Receiver explicitly declined.
    Rejected,
    /// Receiver is busy (already on another call).
    Busy,
}

impl CallStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Initiated => "initiated",
            Self::Ringing => "ringing",
            Self::Answered => "answered",
            Self::Ended => "ended",
            Self::Missed => "missed",
            Self::Rejected => "rejected",
            Self::Busy => "busy",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "initiated" => Some(Self::Initiated),
            "ringing" => Some(Self::Ringing),
            "answered" => Some(Self::Answered),
            "ended" => Some(Self::Ended),
            "missed" => Some(Self::Missed),
            "rejected" => Some(Self::Rejected),
            "busy" => Some(Self::Busy),
            _ => None,
        }
    }

    /// Returns true if this status represents an active/in-progress call.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Initiated | Self::Ringing | Self::Answered)
    }
}

/// A persisted call record, tracking metadata for call history.
/// SDP and ICE data are NOT stored here — they are relayed in real-time
/// via WebSocket / Redis pub-sub only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    pub id: Uuid,
    pub caller_id: Uuid,
    pub receiver_id: Uuid,
    pub call_type: CallType,
    pub status: CallStatus,
    /// Set when the call moves to `answered`.
    pub started_at: Option<DateTime<Utc>>,
    /// Set when the call moves to a terminal state.
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Data required to create a new call record.
#[derive(Debug, Clone)]
pub struct NewCall {
    pub caller_id: Uuid,
    pub receiver_id: Uuid,
    pub call_type: CallType,
}
