use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    NewMessage,
    StoryReaction,
    StoryView,
    ParticipantAdded,
    CallIncoming,
}

impl NotificationType {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::NewMessage => "new_message",
            Self::StoryReaction => "story_reaction",
            Self::StoryView => "story_view",
            Self::ParticipantAdded => "participant_added",
            Self::CallIncoming => "call",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "new_message" => Some(Self::NewMessage),
            "story_reaction" => Some(Self::StoryReaction),
            "story_view" => Some(Self::StoryView),
            "participant_added" => Some(Self::ParticipantAdded),
            "call" => Some(Self::CallIncoming),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: Uuid,
    pub user_id: Uuid,
    pub notification_type: NotificationType,
    pub data: serde_json::Value,
    pub is_read: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewNotification {
    pub user_id: Uuid,
    pub notification_type: NotificationType,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSettings {
    pub is_muted: bool,
    pub muted_until: Option<DateTime<Utc>>,
    pub is_pinned: bool,
    pub pin_order: i32,
    pub is_archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateChatSettings {
    pub is_muted: Option<bool>,
    pub muted_until: Option<DateTime<Utc>>,
    pub is_pinned: Option<bool>,
    pub pin_order: Option<i32>,
    pub is_archived: Option<bool>,
}
