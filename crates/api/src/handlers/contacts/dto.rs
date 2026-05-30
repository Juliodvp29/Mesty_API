use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ContactResponse {
    pub id: String,
    pub contact_id: Option<String>,
    pub phone: String,
    pub nickname: Option<String>,
    pub is_favorite: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    /// Números de teléfono E.164 del dispositivo (el servidor calcula el hash internamente).
    pub phones: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncMatch {
    pub hash: String,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncResponse {
    pub matches: Vec<SyncMatch>,
}

#[derive(Debug, Deserialize)]
pub struct CreateContactRequest {
    pub phone: String,
    pub nickname: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateContactRequest {
    pub nickname: Option<String>,
    pub is_favorite: Option<bool>,
}
