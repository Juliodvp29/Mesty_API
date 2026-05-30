use crate::services::metrics::MetricsExtension;
use axum::{
    Json,
    extract::{
        Extension, Query, State,
        ws::{WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use domain::user::repository::UserRepository;
use futures_util::{SinkExt, StreamExt};
use infrastructure::repositories::chat::PostgresChatRepository;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::WsState;
use super::dto::{WsClientMessage, WsParams};
use crate::error::ApiError;
use crate::middleware::auth::AuthenticatedUser;
use crate::routes::WsRouterState;
use redis::AsyncCommands;
use shared::error::DomainError;

pub async fn create_ws_ticket(
    Extension(auth_user): Extension<AuthenticatedUser>,
    State(state): State<WsRouterState>,
) -> Result<impl IntoResponse, ApiError> {
    let ticket = Uuid::new_v4().to_string();
    let redis_key = format!("ws:ticket:{}", ticket);

    let mut redis = state.ws_state.redis.clone();

    // Store the ticket in Redis mapping to the user ID with a 30 seconds TTL
    let _: () = redis
        .set_ex(&redis_key, auth_user.user_id.to_string(), 30_u64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to store WS ticket in Redis: {:?}", e);
            ApiError(DomainError::Internal(
                "Database connection error".to_string(),
            ))
        })?;

    Ok((StatusCode::CREATED, Json(json!({ "ticket": ticket }))))
}

pub async fn ws_handler(
    Query(params): Query<WsParams>,
    Extension(metrics): Extension<MetricsExtension>,
    State(state): State<WsRouterState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let mut redis = state.ws_state.redis.clone();
    let redis_key = format!("ws:ticket:{}", params.ticket);

    // Look up the user ID associated with this ticket
    let user_id_str: Option<String> = match redis.get(&redis_key).await {
        Ok(val) => val,
        Err(e) => {
            tracing::error!("Failed to retrieve WS ticket from Redis: {:?}", e);
            return Err(ApiError(DomainError::Internal(
                "Database connection error".to_string(),
            )));
        }
    };

    let user_id_str = match user_id_str {
        Some(uid) => uid,
        None => {
            return Err(ApiError(DomainError::Unauthorized(
                "Invalid or expired WebSocket ticket".to_string(),
            )));
        }
    };

    // Delete the ticket immediately to ensure one-time use
    let _: Result<(), _> = redis.del(&redis_key).await;

    let user_id = match Uuid::parse_str(&user_id_str) {
        Ok(id) => id,
        Err(_) => {
            return Err(ApiError(DomainError::Unauthorized(
                "Invalid ticket user identity".to_string(),
            )));
        }
    };

    // Limit concurrent connections per user to prevent DoS attacks
    let active_connections = {
        if let Some(mut entry) = state.ws_state.connections.get_mut(&user_id) {
            entry.retain(|conn| !conn.is_closed());
            entry.len()
        } else {
            0
        }
    };

    if active_connections >= 5 {
        return Err(ApiError(DomainError::Conflict(
            "Too many concurrent WebSocket connections. Maximum is 5.".to_string(),
        )));
    }

    Ok(
        ws.on_upgrade(move |socket| {
            handle_socket(socket, user_id, state.ws_state.clone(), metrics)
        }),
    )
}

async fn handle_socket(
    socket: WebSocket,
    user_id: Uuid,
    ws_state: Arc<WsState>,
    metrics: MetricsExtension,
) {
    // Increment active connections
    metrics.0.read().active_ws_connections.inc();

    let (mut write, mut read) = socket.split();

    let (tx, mut rx) = mpsc::channel::<String>(100);

    {
        let mut user_connections = ws_state.connections.entry(user_id).or_default();
        user_connections.push(tx.clone());
    }

    let redis = ws_state.redis.clone();
    let redis_url = ws_state.redis_url.clone();
    let _: Result<(), redis::RedisError> = redis::pipe()
        .set_ex(format!("presence:{}", user_id), "1", 65_u64)
        .query_async(&mut redis.clone())
        .await;

    let channel = format!("user:{}:events", user_id);
    let tx_clone = tx.clone();

    tokio::spawn(async move {
        let client = match redis::Client::open(redis_url.as_str()) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut pub_sub = match client.get_async_pubsub().await {
            Ok(p) => p,
            Err(_) => return,
        };
        if pub_sub.subscribe(&channel).await.is_err() {
            return;
        }

        let mut stream = pub_sub.on_message();
        while let Some(msg) = stream.next().await {
            let payload: String = msg.get_payload().unwrap_or_default();
            let _ = tx_clone.send(payload).await;
        }
    });

    let user_id_for_read = user_id;
    let connections_for_cleanup = ws_state.connections.clone();
    let redis_for_cleanup = redis.clone();
    let user_repo = ws_state.user_repo.clone();
    let ws_state_clone = ws_state.clone();
    let metrics_for_cleanup = metrics.clone();

    tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(axum::extract::ws::Message::Text(text)) => {
                    if let Ok(client_msg) = serde_json::from_str::<WsClientMessage>(&text) {
                        handle_client_message(
                            client_msg,
                            &tx,
                            &redis,
                            user_id_for_read,
                            ws_state_clone.clone(),
                        )
                        .await;
                    }
                }
                Ok(axum::extract::ws::Message::Ping(_)) => {}
                Ok(axum::extract::ws::Message::Pong(_)) => {}
                Ok(axum::extract::ws::Message::Close(_)) | Err(_) => {
                    break;
                }
                _ => {}
            }
        }

        // Decrement active connections
        metrics_for_cleanup.0.read().active_ws_connections.dec();

        if let Some(mut entry) = connections_for_cleanup.get_mut(&user_id_for_read) {
            entry.retain(|conn| !conn.is_closed());
            if entry.is_empty() {
                drop(entry);
                connections_for_cleanup.remove(&user_id_for_read);
            }
        }

        let redis = redis_for_cleanup.clone();
        let _: Result<(), _> = redis::pipe()
            .del(format!("presence:{}", user_id_for_read))
            .query_async(&mut redis.clone())
            .await;

        let user_id = domain::user::value_objects::UserId(user_id_for_read);
        let _ = user_repo
            .update_last_seen(&user_id, chrono::Utc::now())
            .await;

        tracing::info!("WebSocket disconnected for user {}", user_id_for_read);
    });

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write
                .send(axum::extract::ws::Message::Text(msg))
                .await
                .is_err()
            {
                break;
            }
        }
    });
}

async fn handle_client_message(
    msg: WsClientMessage,
    tx: &mpsc::Sender<String>,
    redis: &redis::aio::ConnectionManager,
    user_id: Uuid,
    ws_state: Arc<WsState>,
) {
    match msg {
        // ---------------------------------------------------------------
        // Chat: Typing indicators
        // ---------------------------------------------------------------
        WsClientMessage::TypingStart { chat_id } => {
            let payload = json!({
                "type": "typing_start",
                "payload": {
                    "chat_id": chat_id,
                    "user_id": user_id
                },
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            let redis1 = redis.clone();
            let redis2 = redis.clone();
            let payload_str = payload.to_string();
            let chat_id1 = chat_id;
            tokio::spawn(async move {
                let mut redis = redis1.clone();
                let _: Result<(), _> = redis::pipe()
                    .publish(format!("chat:{}:events", chat_id1), payload_str)
                    .query_async(&mut redis)
                    .await;
            });

            let user_id_for_timer = user_id;
            let chat_id_for_timer = chat_id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                let payload = json!({
                    "type": "typing_stop",
                    "payload": {
                        "chat_id": chat_id_for_timer,
                        "user_id": user_id_for_timer
                    },
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });

                let payload_str = payload.to_string();
                let mut redis = redis2.clone();
                let _: Result<(), _> = redis::pipe()
                    .publish(format!("chat:{}:events", chat_id_for_timer), payload_str)
                    .query_async(&mut redis)
                    .await;
            });
        }
        WsClientMessage::TypingStop { chat_id } => {
            let payload = json!({
                "type": "typing_stop",
                "payload": {
                    "chat_id": chat_id,
                    "user_id": user_id
                },
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            let redis = redis.clone();
            let payload_str = payload.to_string();
            tokio::spawn(async move {
                let mut redis = redis.clone();
                let _: Result<(), _> = redis::pipe()
                    .publish(format!("chat:{}:events", chat_id), payload_str)
                    .query_async(&mut redis)
                    .await;
            });
        }

        // ---------------------------------------------------------------
        // Chat: Sync
        // ---------------------------------------------------------------
        WsClientMessage::SyncRequest { since } => {
            let chat_repo = ws_state.chat_repo.clone();
            let tx = tx.clone();

            tokio::spawn(async move {
                if let Some(since) = since {
                    match sync_messages_after(chat_repo.as_ref(), user_id, since).await {
                        Ok(messages) => {
                            let payload = json!({
                                "type": "sync_response",
                                "payload": {
                                    "messages": messages
                                },
                                "timestamp": Utc::now().to_rfc3339()
                            });
                            let _ = tx.send(payload.to_string()).await;
                        }
                        Err(_) => {
                            let payload = json!({
                                "type": "sync_response",
                                "payload": {
                                    "messages": Vec::<serde_json::Value>::new()
                                },
                                "timestamp": Utc::now().to_rfc3339()
                            });
                            let _ = tx.send(payload.to_string()).await;
                        }
                    }
                } else {
                    let payload = json!({
                        "type": "sync_response",
                        "payload": {
                            "messages": Vec::<serde_json::Value>::new()
                        },
                        "timestamp": Utc::now().to_rfc3339()
                    });
                    let _ = tx.send(payload.to_string()).await;
                }
            });
        }

        // ---------------------------------------------------------------
        // WebRTC Signaling: CallInitiate
        // ---------------------------------------------------------------
        WsClientMessage::CallInitiate {
            receiver_id,
            call_type,
            offer,
        } => {
            use crate::services::push::{PushNotificationJob, enqueue_push_notification};
            use domain::call::entities::CallType;

            let call_type_domain = match call_type.as_str() {
                "video" => CallType::Video,
                _ => CallType::Audio,
            };

            let call_service = ws_state.call_service.clone();
            let mut redis = redis.clone();
            let ws_state_clone = ws_state.clone();
            let tx_clone = tx.clone();
            let caller_id = user_id;

            tokio::spawn(async move {
                let call = match call_service
                    .initiate_call(caller_id, receiver_id, call_type_domain)
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        // Notify caller of the error (e.g. receiver is busy)
                        let err_payload = json!({
                            "type": "call:error",
                            "payload": { "message": e.to_string() },
                            "timestamp": Utc::now().to_rfc3339()
                        });
                        let _ = tx_clone.send(err_payload.to_string()).await;
                        return;
                    }
                };

                // Build the `call:incoming` event for the receiver
                let incoming = json!({
                    "type": "call:incoming",
                    "payload": {
                        "call_id":   call.id,
                        "caller_id": caller_id,
                        "call_type": call_type_domain.as_db_str(),
                        "offer":     offer,
                        "timestamp": Utc::now().to_rfc3339()
                    }
                });

                // Relay via Redis to receiver's WebSocket channel
                let channel = format!("user:{}:events", receiver_id);
                let _: Result<(), _> = redis::pipe()
                    .publish(&channel, incoming.to_string())
                    .query_async(&mut redis)
                    .await;

                // Mark call as ringing now that we've attempted delivery
                let _ = call_service.mark_ringing(call.id).await;

                // ALWAYS enqueue a push notification so devices wake up
                // (especially iOS VoIP push, offline receivers, etc.)
                let push_job = PushNotificationJob {
                    user_id: receiver_id,
                    notification_type: "call".to_string(),
                    payload: json!({
                        "call_id":    call.id,
                        "caller_id":  caller_id,
                        "call_type":  call_type_domain.as_db_str(),
                        "title":      "Incoming Call",
                        "body":       format!(
                            "You have an incoming {} call",
                            call_type_domain.as_db_str()
                        ),
                    }),
                };

                if let Err(e) =
                    enqueue_push_notification(&mut ws_state_clone.redis.clone(), push_job).await
                {
                    tracing::warn!("Failed to enqueue call push notification: {}", e);
                }

                tracing::info!(
                    call_id = %call.id,
                    caller  = %caller_id,
                    receiver = %receiver_id,
                    "Call initiated and push notification enqueued"
                );
            });
        }

        // ---------------------------------------------------------------
        // WebRTC Signaling: CallAccept
        // ---------------------------------------------------------------
        WsClientMessage::CallAccept { call_id, answer } => {
            let call_service = ws_state.call_service.clone();
            let mut redis = redis.clone();
            let receiver_id = user_id;

            tokio::spawn(async move {
                let call = match call_service.accept_call(call_id, receiver_id).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(call_id = %call_id, "Failed to accept call: {}", e);
                        return;
                    }
                };

                // Relay `call:accepted` to the caller
                let accepted = json!({
                    "type": "call:accepted",
                    "payload": {
                        "call_id":     call_id,
                        "receiver_id": receiver_id,
                        "answer":      answer,
                        "timestamp":   Utc::now().to_rfc3339()
                    }
                });

                let channel = format!("user:{}:events", call.caller_id);
                let _: Result<(), _> = redis::pipe()
                    .publish(&channel, accepted.to_string())
                    .query_async(&mut redis)
                    .await;

                tracing::info!(call_id = %call_id, "Call accepted");
            });
        }

        // ---------------------------------------------------------------
        // WebRTC Signaling: CallReject
        // ---------------------------------------------------------------
        WsClientMessage::CallReject { call_id, reason } => {
            use domain::call::entities::CallStatus;

            let call_service = ws_state.call_service.clone();
            let mut redis = redis.clone();
            let user_id_clone = user_id;

            tokio::spawn(async move {
                let reason_str = reason.unwrap_or_else(|| "rejected".to_string());
                let terminal_status = if reason_str == "busy" {
                    CallStatus::Busy
                } else {
                    CallStatus::Rejected
                };

                let call = match call_service
                    .end_call(call_id, user_id_clone, terminal_status)
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(call_id = %call_id, "Failed to reject call: {}", e);
                        return;
                    }
                };

                // Notify the caller that the call was rejected/busy
                let rejected = json!({
                    "type": "call:rejected",
                    "payload": {
                        "call_id":     call_id,
                        "receiver_id": user_id_clone,
                        "reason":      reason_str,
                        "timestamp":   Utc::now().to_rfc3339()
                    }
                });

                let channel = format!("user:{}:events", call.caller_id);
                let _: Result<(), _> = redis::pipe()
                    .publish(&channel, rejected.to_string())
                    .query_async(&mut redis)
                    .await;

                tracing::info!(call_id = %call_id, "Call rejected");
            });
        }

        // ---------------------------------------------------------------
        // WebRTC Signaling: CallIceCandidate (relay only — NOT persisted)
        // ---------------------------------------------------------------
        WsClientMessage::CallIceCandidate {
            call_id,
            receiver_id,
            candidate,
        } => {
            let sender_id = user_id;
            let mut redis = redis.clone();

            tokio::spawn(async move {
                let relay = json!({
                    "type": "call:ice-candidate",
                    "payload": {
                        "call_id":   call_id,
                        "sender_id": sender_id,
                        "candidate": candidate,
                        "timestamp": Utc::now().to_rfc3339()
                    }
                });

                let channel = format!("user:{}:events", receiver_id);
                let _: Result<(), _> = redis::pipe()
                    .publish(&channel, relay.to_string())
                    .query_async(&mut redis)
                    .await;
            });
        }

        // ---------------------------------------------------------------
        // WebRTC Signaling: CallHangup
        // ---------------------------------------------------------------
        WsClientMessage::CallHangup { call_id } => {
            use domain::call::entities::CallStatus;

            let call_service = ws_state.call_service.clone();
            let mut redis = redis.clone();
            let user_id_clone = user_id;

            tokio::spawn(async move {
                // Determine the right terminal status based on the call state
                let call_before = match call_service
                    .end_call(call_id, user_id_clone, CallStatus::Ended)
                    .await
                {
                    Ok(c) => c,
                    Err(_) => {
                        // Try with Missed (if the receiver hasn't answered yet)
                        match call_service
                            .end_call(call_id, user_id_clone, CallStatus::Missed)
                            .await
                        {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::warn!(
                                    call_id = %call_id,
                                    "Failed to hang up call: {}",
                                    e
                                );
                                return;
                            }
                        }
                    }
                };

                let final_status = call_before.status.as_db_str();

                // Notify BOTH participants (caller and receiver)
                let ended = json!({
                    "type": "call:ended",
                    "payload": {
                        "call_id":  call_id,
                        "ended_by": user_id_clone,
                        "status":   final_status,
                        "timestamp": Utc::now().to_rfc3339()
                    }
                });
                let ended_str = ended.to_string();

                let peer_id = if call_before.caller_id == user_id_clone {
                    call_before.receiver_id
                } else {
                    call_before.caller_id
                };

                let channel = format!("user:{}:events", peer_id);
                let _: Result<(), _> = redis::pipe()
                    .publish(&channel, ended_str)
                    .query_async(&mut redis)
                    .await;

                tracing::info!(
                    call_id  = %call_id,
                    ended_by = %user_id_clone,
                    status   = %final_status,
                    "Call hung up"
                );
            });
        }
    }
}

async fn sync_messages_after(
    chat_repo: &PostgresChatRepository,
    user_id: Uuid,
    since: DateTime<Utc>,
) -> Result<Vec<serde_json::Value>, ()> {
    use domain::chat::repository::ChatRepository;

    let messages = chat_repo
        .list_messages_since(user_id, since, 100)
        .await
        .map_err(|_| ())?;

    let mut all_messages = Vec::new();

    for msg in messages {
        all_messages.push(serde_json::json!({
            "chat_id": msg.chat_id,
            "message_id": msg.id,
            "sender_id": msg.sender_id,
            "content_encrypted": msg.content_encrypted,
            "content_iv": msg.content_iv,
            "message_type": msg.message_type,
            "created_at": msg.created_at,
        }));
    }

    Ok(all_messages)
}
