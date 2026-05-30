use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use crate::middleware::auth::AuthenticatedUser;
use domain::contact::value_objects::ContactId;
use domain::user::value_objects::UserId as DomainUserId;
use infrastructure::repositories::contact::PostgresContactRepository;
use infrastructure::repositories::user::PostgresUserRepository;
use shared::error::DomainError;
use shared::hash::{hash_phone, hash_phone_sha256};

use super::dto::{
    ContactResponse, CreateContactRequest, SyncMatch, SyncRequest, SyncResponse,
    UpdateContactRequest,
};

#[derive(Clone)]
pub struct ContactsState {
    pub contact_repo: Arc<PostgresContactRepository>,
    pub user_repo: Arc<PostgresUserRepository>,
    pub phone_hash_secret: String,
}

pub async fn sync_contacts(
    State(state): State<ContactsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<SyncRequest>,
) -> Result<Response, ApiError> {
    if req.phones.is_empty() || req.phones.len() > 1000 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Phone list must be between 1 and 1000 items" })),
        )
            .into_response());
    }

    // Calcular HMAC-SHA256 de cada teléfono para buscar en Redis/Postgres.
    // El cliente nunca conoce ni el secreto ni los hashes almacenados.
    let secret = state.phone_hash_secret.as_bytes();
    let hmac_hashes: Vec<String> = req.phones.iter().map(|p| hash_phone(p, secret)).collect();

    let matches = state
        .contact_repo
        .sync_contacts(&auth.user_id, &hmac_hashes)
        .await?;

    // El campo `hash` de la respuesta usa SHA-256 plano para que el cliente pueda
    // mapear la respuesta a sus contactos locales sin conocer el secreto del servidor.
    let response: Vec<SyncMatch> = matches
        .into_iter()
        .map(|(hmac_hash, user_id, username, display_name)| {
            // Localizar el teléfono original que produjo este hmac_hash
            let original_phone = req
                .phones
                .iter()
                .zip(hmac_hashes.iter())
                .find(|(_, h)| *h == &hmac_hash)
                .map(|(p, _)| p.as_str())
                .unwrap_or("");
            SyncMatch {
                hash: hash_phone_sha256(original_phone),
                user_id,
                username,
                display_name,
            }
        })
        .collect();

    Ok((StatusCode::OK, Json(SyncResponse { matches: response })).into_response())
}

pub async fn list_contacts(
    State(state): State<ContactsState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    let contacts = state
        .contact_repo
        .find_all_by_owner_owned(&auth.user_id)
        .await?;

    let response: Vec<ContactResponse> = contacts
        .into_iter()
        .map(|c| ContactResponse {
            id: c.id.0.to_string(),
            contact_id: c.contact_id.map(|id| id.0.to_string()),
            phone: c.phone.as_str().to_string(),
            nickname: c.nickname,
            is_favorite: c.is_favorite,
            created_at: c.created_at.to_rfc3339(),
        })
        .collect();

    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn create_contact(
    State(state): State<ContactsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CreateContactRequest>,
) -> Result<Response, ApiError> {
    let phone_str = req.phone.trim();
    if phone_str.is_empty() || !phone_str.starts_with('+') {
        return Err(DomainError::Validation("Invalid phone number format".to_string()).into());
    }

    let existing = state
        .contact_repo
        .find_by_owner_and_phone_owned(&auth.user_id, phone_str)
        .await?;

    if existing.is_some() {
        return Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Contact already exists" })),
        )
            .into_response());
    }

    let contact_id = state.user_repo.find_id_by_phone(phone_str).await?;

    state
        .contact_repo
        .create_contact_raw(auth.user_id, phone_str, req.nickname.as_deref(), contact_id)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(ContactResponse {
            id: Uuid::new_v4().to_string(),
            contact_id: contact_id.map(|id| id.to_string()),
            phone: phone_str.to_string(),
            nickname: req.nickname,
            is_favorite: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        }),
    )
        .into_response())
}

pub async fn update_contact(
    State(state): State<ContactsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(contact_id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Response, ApiError> {
    let contact = state
        .contact_repo
        .find_by_id_owned(&contact_id)
        .await?
        .ok_or_else(|| DomainError::NotFound("Contact not found".to_string()))?;

    if contact.owner_id != DomainUserId(auth.user_id) {
        return Err(DomainError::Unauthorized("Not authorized".to_string()).into());
    }

    let mut updated = contact;
    if let Some(nickname) = req.nickname {
        updated.nickname = Some(nickname);
    }
    if let Some(is_favorite) = req.is_favorite {
        updated.set_favorite(is_favorite);
    }

    state.contact_repo.update_contact(&updated).await?;

    Ok((
        StatusCode::OK,
        Json(ContactResponse {
            id: updated.id.0.to_string(),
            contact_id: updated.contact_id.map(|id| id.0.to_string()),
            phone: updated.phone.as_str().to_string(),
            nickname: updated.nickname,
            is_favorite: updated.is_favorite,
            created_at: updated.created_at.to_rfc3339(),
        }),
    )
        .into_response())
}

pub async fn delete_contact(
    State(state): State<ContactsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(contact_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let contact = state
        .contact_repo
        .find_by_id_owned(&contact_id)
        .await?
        .ok_or_else(|| DomainError::NotFound("Contact not found".to_string()))?;

    if contact.owner_id.0 != auth.user_id {
        return Err(DomainError::Unauthorized("Not authorized".to_string()).into());
    }

    state
        .contact_repo
        .delete_contact(&ContactId(contact_id))
        .await?;

    Ok((StatusCode::NO_CONTENT, Json(serde_json::json!({}))).into_response())
}
