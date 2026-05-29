use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use domain::chat::entity::{Chat, ChatMessage, ChatPreview};
use domain::chat::notifications::{Notification, NotificationCursor, UpdateChatSettings};
use domain::chat::repository::{
    ChatCursor, ChatRepository, MessageCursor, MessageDirection, NewMessage,
};
use redis::AsyncCommands;
use shared::error::DomainError;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use crate::handlers::chats::dto::{
    AddReactionRequest, ChatCursorDto, ChatPreviewResponse, ChatResponse, ChatSettingsResponse,
    CreateChatRequest, DeleteChatResponse, DeleteMessageResponse, DeleteReadNotificationsResponse,
    EditMessageRequest, EditMessageResponse, ListChatsQuery, ListChatsResponse, ListMessagesQuery,
    ListMessagesResponse, ListNotificationsQuery, ListNotificationsResponse,
    MarkAllNotificationsReadResponse, MarkMessagesReadRequest, MarkMessagesReadResponse,
    MarkNotificationReadResponse, MessageCursorDto, MessageResponse, NotificationCursorDto,
    NotificationResponse, ReactionResponse, RemoveReactionResponse, SendMessageRequest,
    UpdateChatRequest, UpdateChatResponse, UpdateChatSettingsRequest,
};
use crate::middleware::auth::AuthenticatedUser;
use crate::services::metrics::MetricsExtension;
use crate::services::push::{PRESENCE_KEY_PREFIX, PushNotificationJob, enqueue_push_notification};
use crate::services::rate_limit::check_rate_limit;
use infrastructure::repositories::chat::PostgresChatRepository;

#[derive(Clone)]
pub struct ChatsState {
    pub chat_repo: Arc<PostgresChatRepository>,
    pub redis: redis::aio::ConnectionManager,
}

pub async fn create_chat(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CreateChatRequest>,
) -> Result<Response, ApiError> {
    let chat = match req {
        CreateChatRequest::Private { participant_id } => {
            if participant_id == auth.user_id {
                return Err(ApiError(DomainError::Validation(
                    "participant_id cannot be the authenticated user".to_string(),
                )));
            }
            state
                .chat_repo
                .create_private_chat(auth.user_id, participant_id)
                .await?
        }
        CreateChatRequest::Group {
            name,
            participant_ids,
        } => {
            if name.trim().is_empty() {
                return Err(ApiError(DomainError::Validation(
                    "group name cannot be empty".to_string(),
                )));
            }
            state
                .chat_repo
                .create_group_chat(auth.user_id, name.trim(), &participant_ids)
                .await?
        }
    };

    Ok((StatusCode::CREATED, Json(chat_to_response(chat))).into_response())
}

pub async fn get_chat(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let chat = state
        .chat_repo
        .get_chat_for_user(auth.user_id, chat_id)
        .await?
        .ok_or_else(|| DomainError::NotFound("chat not found".to_string()))?;

    Ok((StatusCode::OK, Json(chat_to_response(chat))).into_response())
}

pub async fn list_chats(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Query(query): Query<ListChatsQuery>,
) -> Result<Response, ApiError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let decoded_cursor = decode_cursor(query.cursor.as_deref())?;
    let fetch_limit = limit + 1;

    let mut items = state
        .chat_repo
        .list_chats_for_user(auth.user_id, decoded_cursor, fetch_limit)
        .await?;

    let has_more = items.len() as i64 > limit;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = items.last().map(build_cursor).transpose()?;

    let response = ListChatsResponse {
        items: items.into_iter().map(preview_to_response).collect(),
        next_cursor,
        has_more,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn send_message(
    State(state): State<ChatsState>,
    Extension(metrics): Extension<MetricsExtension>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Response, ApiError> {
    // Apply rate limit: maximum 15 messages every 10 seconds per user
    let mut redis = state.redis.clone();
    let rate_key = format!("msg:{}", auth.user_id);
    let allowed = check_rate_limit(&mut redis, &rate_key, 15, 10)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    if !allowed {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "Too many messages sent. Please slow down." })),
        )
            .into_response());
    }

    // Record metric
    metrics.0.read().messages_sent_total.inc();
    let message = state
        .chat_repo
        .send_message(
            auth.user_id,
            chat_id,
            NewMessage {
                content_encrypted: req.content_encrypted,
                content_iv: req.content_iv,
                message_type: req.message_type,
                reply_to_id: req.reply_to_id,
                is_forwarded: req.is_forwarded,
                metadata: req.metadata,
            },
        )
        .await?;

    publish_message_event(&state, &message, auth.user_id).await?;

    Ok((StatusCode::CREATED, Json(message_to_response(message))).into_response())
}

pub async fn list_messages(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Response, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 50);
    let direction = match query.direction.as_deref().unwrap_or("before") {
        "before" => MessageDirection::Before,
        "after" => MessageDirection::After,
        _ => {
            return Err(ApiError(DomainError::Validation(
                "direction must be before or after".to_string(),
            )));
        }
    };

    let decoded_cursor = decode_message_cursor(query.cursor.as_deref())?;
    let fetch_limit = limit + 1;
    let mut items = state
        .chat_repo
        .list_messages(
            auth.user_id,
            chat_id,
            decoded_cursor,
            direction,
            fetch_limit,
        )
        .await?;

    let has_more = items.len() as i64 > limit;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = items.last().map(build_message_cursor).transpose()?;
    let response = ListMessagesResponse {
        items: items.into_iter().map(message_to_response).collect(),
        next_cursor,
        has_more,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn decode_cursor(raw: Option<&str>) -> Result<Option<ChatCursor>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| DomainError::Validation("invalid cursor encoding".to_string()))?;
    let dto: ChatCursorDto = serde_json::from_slice(&decoded)
        .map_err(|_| DomainError::Validation("invalid cursor payload".to_string()))?;

    Ok(Some(ChatCursor {
        is_pinned: dto.is_pinned,
        last_message_at: dto.last_message_at,
        chat_id: dto.chat_id,
    }))
}

fn build_cursor(item: &ChatPreview) -> Result<String, ApiError> {
    let dto = ChatCursorDto {
        is_pinned: item.is_pinned,
        last_message_at: item.last_message_at,
        chat_id: item.chat_id,
    };
    let encoded = serde_json::to_vec(&dto)
        .map_err(|e| DomainError::Internal(format!("failed to serialize cursor: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

fn chat_to_response(chat: Chat) -> ChatResponse {
    ChatResponse {
        id: chat.id,
        chat_type: chat.chat_type.as_db_str().to_string(),
        name: chat.name,
        avatar_url: chat.avatar_url,
        created_by: chat.created_by,
        created_at: chat.created_at,
    }
}

fn preview_to_response(preview: ChatPreview) -> ChatPreviewResponse {
    ChatPreviewResponse {
        chat_id: preview.chat_id,
        chat_type: preview.chat_type.as_db_str().to_string(),
        name: preview.name,
        avatar_url: preview.avatar_url,
        last_message_id: preview.last_message_id,
        last_message_encrypted: preview.last_message_encrypted,
        last_sender_id: preview.last_sender_id,
        last_message_at: preview.last_message_at,
        is_pinned: preview.is_pinned,
        pin_order: preview.pin_order,
        is_muted: preview.is_muted,
        is_archived: preview.is_archived,
        unread_count: preview.unread_count,
    }
}

fn message_to_response(message: ChatMessage) -> MessageResponse {
    MessageResponse {
        id: message.id,
        chat_id: message.chat_id,
        sender_id: message.sender_id,
        reply_to_id: message.reply_to_id,
        content_encrypted: message.content_encrypted,
        content_iv: message.content_iv,
        message_type: message.message_type,
        metadata: message.metadata,
        is_forwarded: message.is_forwarded,
        created_at: message.created_at,
        edited_at: message.edited_at,
        deleted_at: message.deleted_at,
    }
}

fn decode_message_cursor(raw: Option<&str>) -> Result<Option<MessageCursor>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| DomainError::Validation("invalid message cursor encoding".to_string()))?;
    let dto: MessageCursorDto = serde_json::from_slice(&decoded)
        .map_err(|_| DomainError::Validation("invalid message cursor payload".to_string()))?;
    Ok(Some(MessageCursor {
        created_at: dto.created_at,
        message_id: dto.message_id,
    }))
}

fn build_message_cursor(item: &ChatMessage) -> Result<String, ApiError> {
    let dto = MessageCursorDto {
        created_at: item.created_at,
        message_id: item.id,
    };
    let encoded = serde_json::to_vec(&dto)
        .map_err(|e| DomainError::Internal(format!("failed to serialize message cursor: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

async fn publish_message_event(
    state: &ChatsState,
    message: &ChatMessage,
    sender_id: Uuid,
) -> Result<(), ApiError> {
    let mut redis = state.redis.clone();
    let channel = format!("chat:{}:events", message.chat_id);
    let payload = serde_json::json!({
        "type": "new_message",
        "chat_id": message.chat_id,
        "message_id": message.id,
        "sender_id": message.sender_id,
        "created_at": message.created_at,
    })
    .to_string();

    let _: i64 = redis
        .publish(channel, payload)
        .await
        .map_err(|e| DomainError::Internal(format!("failed to publish message event: {e}")))?;

    let participants = state
        .chat_repo
        .get_chat_participants(message.chat_id)
        .await
        .map_err(|e| DomainError::Internal(format!("failed to get participants: {e}")))?;

    for participant_id in participants {
        if participant_id != sender_id {
            let user_channel = format!("user:{}:events", participant_id);
            let user_payload = serde_json::json!({
                "type": "new_message",
                "payload": {
                    "chat_id": message.chat_id,
                    "message_id": message.id,
                    "sender_id": message.sender_id,
                    "content_encrypted": message.content_encrypted,
                    "content_iv": message.content_iv,
                    "message_type": message.message_type,
                },
                "timestamp": message.created_at.to_rfc3339(),
            })
            .to_string();

            let _: i64 = redis
                .publish(user_channel, user_payload)
                .await
                .map_err(|e| DomainError::Internal(format!("failed to publish user event: {e}")))?;

            // 1. Check if the user is online
            let presence_key = format!("{}{}", PRESENCE_KEY_PREFIX, participant_id);
            let is_online: bool = redis.exists(&presence_key).await.unwrap_or(false);

            if !is_online {
                // 2. Revisar preferencias (mute)
                let settings = state
                    .chat_repo
                    .get_chat_settings(participant_id, message.chat_id)
                    .await
                    .unwrap_or(None)
                    .unwrap_or_default();

                let is_muted = settings.is_muted
                    || settings
                        .muted_until
                        .is_some_and(|until| until > chrono::Utc::now());

                if !is_muted {
                    // 3. Enqueue Push Notification
                    let job = PushNotificationJob {
                        user_id: participant_id,
                        notification_type: "new_message".to_string(),
                        payload: serde_json::json!({
                            "chat_id": message.chat_id,
                            "message_id": message.id,
                            "sender_id": message.sender_id,
                        }),
                    };

                    if let Err(e) = enqueue_push_notification(&mut redis, job).await {
                        tracing::error!("Failed to enqueue push notification: {}", e);
                    }
                }
            }
        }
    }

    Ok(())
}

pub async fn mark_messages_read(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
    Json(req): Json<MarkMessagesReadRequest>,
) -> Result<Response, ApiError> {
    // Verify user is participant in the chat
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    // Mark messages as read
    let updated_count = state
        .chat_repo
        .mark_messages_read(auth.user_id, chat_id, req.up_to)
        .await?;

    // Publish read event to Redis
    publish_messages_read_event(&state, chat_id, auth.user_id, req.up_to).await?;

    let response = MarkMessagesReadResponse { updated_count };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn add_reaction(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path((chat_id, message_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<AddReactionRequest>,
) -> Result<Response, ApiError> {
    // Verify user is participant in the chat
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    // Verify message exists and belongs to chat
    state
        .chat_repo
        .verify_message_in_chat(message_id, chat_id)
        .await?;

    // Add reaction
    let reaction = state
        .chat_repo
        .add_reaction(message_id, auth.user_id, req.reaction)
        .await?;

    // Publish reaction event to Redis
    publish_reaction_event(
        &state,
        chat_id,
        message_id,
        auth.user_id,
        &reaction.reaction,
        "add",
    )
    .await?;

    let response = ReactionResponse {
        id: reaction.id,
        message_id: reaction.message_id,
        user_id: reaction.user_id,
        reaction: reaction.reaction,
        created_at: reaction.created_at,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

pub async fn remove_reaction(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path((chat_id, message_id, emoji)): Path<(Uuid, Uuid, String)>,
) -> Result<Response, ApiError> {
    // Decode emoji from URL
    let reaction_emoji = urlencoding::decode(&emoji)
        .map_err(|_| DomainError::Validation("invalid emoji encoding".to_string()))?
        .into_owned();

    // Verify user is participant in the chat
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    // Verify message exists and belongs to chat
    state
        .chat_repo
        .verify_message_in_chat(message_id, chat_id)
        .await?;

    // Remove reaction
    let removed = state
        .chat_repo
        .remove_reaction(message_id, auth.user_id, &reaction_emoji)
        .await?;

    if removed {
        // Publish reaction event to Redis
        publish_reaction_event(
            &state,
            chat_id,
            message_id,
            auth.user_id,
            &reaction_emoji,
            "remove",
        )
        .await?;
    }

    let response = RemoveReactionResponse { removed };
    Ok((StatusCode::OK, Json(response)).into_response())
}

async fn publish_messages_read_event(
    state: &ChatsState,
    chat_id: Uuid,
    user_id: Uuid,
    up_to: chrono::DateTime<chrono::Utc>,
) -> Result<(), ApiError> {
    let mut redis = state.redis.clone();
    let channel = format!("chat:{}:events", chat_id);
    let payload = serde_json::json!({
        "type": "messages_read",
        "chat_id": chat_id,
        "user_id": user_id,
        "up_to": up_to,
    })
    .to_string();

    let _: i64 = redis.publish(channel, payload).await.map_err(|e| {
        DomainError::Internal(format!("failed to publish messages_read event: {e}"))
    })?;
    Ok(())
}

async fn publish_reaction_event(
    state: &ChatsState,
    chat_id: Uuid,
    message_id: Uuid,
    user_id: Uuid,
    reaction: &str,
    action: &str,
) -> Result<(), ApiError> {
    let mut redis = state.redis.clone();
    let channel = format!("chat:{}:events", chat_id);
    let payload = serde_json::json!({
        "type": "reaction",
        "chat_id": chat_id,
        "message_id": message_id,
        "user_id": user_id,
        "reaction": reaction,
        "action": action,
    })
    .to_string();

    let _: i64 = redis
        .publish(channel, payload)
        .await
        .map_err(|e| DomainError::Internal(format!("failed to publish reaction event: {e}")))?;
    Ok(())
}

pub async fn update_chat(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
    Json(req): Json<UpdateChatRequest>,
) -> Result<Response, ApiError> {
    let chat = state
        .chat_repo
        .update_chat(
            auth.user_id,
            chat_id,
            req.name,
            req.description,
            req.avatar_url,
        )
        .await?;

    let response = UpdateChatResponse {
        id: chat.id,
        chat_type: chat.chat_type.as_db_str().to_string(),
        name: chat.name,
        description: chat.description,
        avatar_url: chat.avatar_url,
        updated_at: chat.created_at,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn delete_chat(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    state.chat_repo.delete_chat(auth.user_id, chat_id).await?;

    let response = DeleteChatResponse { deleted: true };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn edit_message(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path((chat_id, message_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<EditMessageRequest>,
) -> Result<Response, ApiError> {
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    let message = state
        .chat_repo
        .edit_message(
            auth.user_id,
            message_id,
            req.content_encrypted,
            req.content_iv,
        )
        .await?;

    let response = EditMessageResponse {
        id: message.id,
        chat_id: message.chat_id,
        content_encrypted: message.content_encrypted,
        content_iv: message.content_iv,
        edited_at: message.edited_at.unwrap_or(message.created_at),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn delete_message(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path((chat_id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    state
        .chat_repo
        .delete_message(auth.user_id, message_id)
        .await?;

    let response = DeleteMessageResponse { deleted: true };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn list_notifications(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Query(query): Query<ListNotificationsQuery>,
) -> Result<Response, ApiError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let decoded_cursor = decode_notification_cursor(query.cursor.as_deref())?;
    let fetch_limit = limit + 1;

    let mut items = state
        .chat_repo
        .list_notifications(auth.user_id, decoded_cursor, fetch_limit)
        .await?;

    let has_more = items.len() as i64 > limit;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = items.last().map(build_notification_cursor).transpose()?;

    let response = ListNotificationsResponse {
        items: items.into_iter().map(notification_to_response).collect(),
        next_cursor,
        has_more,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn mark_notification_read(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(notification_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    state
        .chat_repo
        .mark_notification_read(auth.user_id, notification_id)
        .await?;

    let response = MarkNotificationReadResponse { updated: true };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn mark_all_notifications_read(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    let count = state
        .chat_repo
        .mark_all_notifications_read(auth.user_id)
        .await?;

    let response = MarkAllNotificationsReadResponse {
        updated_count: count,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn delete_read_notifications(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    let count = state
        .chat_repo
        .delete_read_notifications(auth.user_id)
        .await?;

    let response = DeleteReadNotificationsResponse {
        deleted_count: count,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn update_chat_settings(
    State(state): State<ChatsState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(chat_id): Path<Uuid>,
    Json(req): Json<UpdateChatSettingsRequest>,
) -> Result<Response, ApiError> {
    state
        .chat_repo
        .verify_participant(auth.user_id, chat_id)
        .await?;

    let settings = state
        .chat_repo
        .update_chat_settings(
            auth.user_id,
            chat_id,
            UpdateChatSettings {
                is_muted: req.is_muted,
                muted_until: req.muted_until,
                is_pinned: req.is_pinned,
                pin_order: req.pin_order,
                is_archived: req.is_archived,
            },
        )
        .await?;

    let response = ChatSettingsResponse {
        is_muted: settings.is_muted,
        muted_until: settings.muted_until,
        is_pinned: settings.is_pinned,
        pin_order: settings.pin_order,
        is_archived: settings.is_archived,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn decode_notification_cursor(raw: Option<&str>) -> Result<Option<NotificationCursor>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| DomainError::Validation("invalid notification cursor encoding".to_string()))?;
    let dto: NotificationCursorDto = serde_json::from_slice(&decoded)
        .map_err(|_| DomainError::Validation("invalid notification cursor payload".to_string()))?;

    Ok(Some(NotificationCursor {
        created_at: dto.created_at,
        id: dto.id,
    }))
}

fn build_notification_cursor(item: &Notification) -> Result<String, ApiError> {
    let dto = NotificationCursorDto {
        created_at: item.created_at,
        id: item.id,
    };
    let encoded = serde_json::to_vec(&dto).map_err(|e| {
        DomainError::Internal(format!("failed to serialize notification cursor: {e}"))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(encoded))
}

fn notification_to_response(notification: Notification) -> NotificationResponse {
    NotificationResponse {
        id: notification.id,
        notification_type: notification.notification_type.as_db_str().to_string(),
        data: notification.data,
        is_read: notification.is_read,
        read_at: notification.read_at,
        created_at: notification.created_at,
    }
}
