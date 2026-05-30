use super::value_objects::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: Option<Username>,
    pub phone: PhoneNumber,
    pub phone_hash: String,
    pub email: Option<Email>,
    pub status_text: String,
    pub two_fa_enabled: bool,
    pub two_fa_secret: Option<String>,
    pub is_active: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

use shared::hash::hash_phone;

impl User {
    pub fn new(
        username: Option<Username>,
        phone: PhoneNumber,
        email: Option<Email>,
        phone_hash_secret: &[u8],
    ) -> Self {
        let now = Utc::now();
        let phone_hash = hash_phone(phone.as_str(), phone_hash_secret);
        Self {
            id: UserId::new(),
            username,
            phone,
            phone_hash,
            email,
            status_text: String::new(),
            two_fa_enabled: false,
            two_fa_secret: None,
            is_active: true,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn soft_delete(&mut self) {
        self.deleted_at = Some(Utc::now());
    }
}
