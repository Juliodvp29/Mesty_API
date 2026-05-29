use crate::handlers::auth::AuthState;
use crate::middleware::logging::logging_middleware;
use crate::middleware::security::security_headers_middleware;
use crate::services::metrics::{MetricsExtension, SharedMetrics, metrics_handler};
use axum::{
    Router,
    http::{HeaderValue, Method, header},
    middleware,
    routing::{delete, get, patch, post},
};
use tower_http::cors::CorsLayer;

use crate::handlers::attachments::{AttachmentsState, confirm_attachment, create_upload_url};
use crate::handlers::auth::{
    delete_session, list_sessions, login, login_verify, logout, recover, recover_verify, refresh,
    register, two_fa_setup, two_fa_setup_verify, two_fa_verify, verify_phone,
};
use crate::handlers::blocks::handlers::{BlocksState, block_user, list_blocked, unblock_user};
use crate::handlers::calls::{CallsState, get_turn_credentials};
use crate::handlers::chats::{
    ChatsState, add_reaction, create_chat, delete_chat, delete_message, delete_read_notifications,
    edit_message, get_chat, list_chats, list_messages, list_notifications,
    mark_all_notifications_read, mark_messages_read, mark_notification_read, remove_reaction,
    send_message, update_chat, update_chat_settings,
};
use crate::handlers::contacts::handlers::{
    ContactsState, create_contact, delete_contact, list_contacts, sync_contacts, update_contact,
};
use crate::handlers::groups::{
    add_participant, create_invite_link, delete_invite_link, join_by_slug, list_participants,
    remove_participant, rotate_group_key, transfer_ownership, update_participant_role,
};
use crate::handlers::keys::{
    KeysState, get_fingerprint, get_key_bundle, get_my_prekey_count, upload_keys, upload_prekeys,
};
use crate::handlers::stories::StoryRepo;
use crate::handlers::stories::{
    StoriesState, create_story, delete_story, get_story_views, list_my_stories, list_stories,
    react_to_story, view_story,
};
use crate::handlers::users::handlers::{
    UsersState, get_my_profile, get_user_profile, search_users,
};
use crate::handlers::ws::{WsState, ws_handler};
use crate::middleware::auth::{AuthMiddlewareState, auth_middleware};
use crate::services::jwt::JwtService;
use crate::services::otp::OtpService;
use crate::services::storage::S3StorageService;
use domain::call::{CallRepository, CallService};
use infrastructure::cache::ProfileCache;
use infrastructure::repositories::call::PostgresCallRepository;
use infrastructure::repositories::chat::PostgresChatRepository;
use infrastructure::repositories::contact::PostgresContactRepository;
use infrastructure::repositories::keys::PostgresKeyRepository;
use infrastructure::repositories::stories::PostgresStoryRepository;
use infrastructure::repositories::user::PostgresUserRepository;
use redis::aio::ConnectionManager;
use shared::config::Config;
use std::sync::Arc;

#[derive(Clone)]
pub struct WsRouterState {
    pub jwt_service: Arc<JwtService>,
    pub ws_state: Arc<WsState>,
}

pub fn create_router(
    config: &Config,
    db_pool: sqlx::PgPool,
    redis_manager: ConnectionManager,
    metrics: SharedMetrics,
) -> Router {
    let user_repo = Arc::new(PostgresUserRepository::new(
        db_pool.clone(),
        Some(redis_manager.clone()),
    ));
    let chat_repo = Arc::new(PostgresChatRepository::new(db_pool.clone()));
    let key_repo = Arc::new(PostgresKeyRepository::new(db_pool.clone()));

    let otp_service = Arc::new(OtpService::new(redis_manager.clone(), 600));

    let jwt_service = Arc::new(JwtService::new(
        config.jwt.private_key.clone(),
        config.jwt.public_key.clone(),
        config.jwt.access_ttl_seconds,
        config.jwt.refresh_ttl_seconds,
        Some(redis_manager.clone()),
    ));

    let auth_middleware_state = Arc::new(AuthMiddlewareState {
        jwt_service: jwt_service.clone(),
        redis: redis_manager.clone(),
        user_repo: user_repo.clone(),
    });

    let auth_state = AuthState {
        user_repo: user_repo.clone(),
        otp_service: otp_service.clone(),
        jwt_service: jwt_service.clone(),
    };

    let keys_state = KeysState {
        key_repo: key_repo.clone(),
    };

    let chats_state = ChatsState {
        chat_repo: chat_repo.clone(),
        redis: redis_manager.clone(),
    };

    let storage = Arc::new(S3StorageService::new(&config.s3));

    // Apply S3 lifecycle rules on startup to ensure story files are cleaned up
    let storage_clone = storage.clone();
    tokio::spawn(async move {
        if let Err(e) = storage_clone.apply_lifecycle_rules().await {
            tracing::error!("Failed to apply init-time S3 lifecycle rules: {}", e);
        }
    });

    let attachments_state = AttachmentsState {
        chat_repo: chat_repo.clone(),
        storage,
    };

    let story_repo: Arc<dyn StoryRepo> = Arc::new(PostgresStoryRepository::new(db_pool.clone()));
    let stories_state = StoriesState {
        story_repo: story_repo.clone(),
        chat_repo: chat_repo.clone(),
        redis: redis_manager.clone(),
    };

    let contact_repo = Arc::new(PostgresContactRepository::new(
        db_pool.clone(),
        Some(redis_manager.clone()),
    ));
    let contacts_state = ContactsState {
        contact_repo: contact_repo.clone(),
        user_repo: user_repo.clone(),
    };

    let blocks_state = BlocksState {
        user_repo: user_repo.clone(),
    };

    let profile_cache = Arc::new(ProfileCache::new(redis_manager.clone()));

    let users_state = UsersState {
        user_repo: user_repo.clone(),
        otp_service: otp_service.clone(),
        profile_cache: Some(profile_cache),
    };

    let ws_state = Arc::new(WsState {
        connections: Arc::new(dashmap::DashMap::new()),
        redis: redis_manager.clone(),
        redis_url: config.redis.url.clone(),
        user_repo: user_repo.clone(),
        chat_repo: chat_repo.clone(),
        call_service: {
            let call_repo = Arc::new(PostgresCallRepository::new(db_pool.clone()));
            Arc::new(CallService::new(call_repo as Arc<dyn CallRepository>))
        },
    });

    let calls_state = CallsState {
        turn_config: Arc::new(config.turn.clone()),
    };

    let ws_router_state = WsRouterState {
        jwt_service: jwt_service.clone(),
        ws_state: ws_state.clone(),
    };

    let protected_auth_routes = Router::new()
        .route("/auth/2fa/setup", post(two_fa_setup))
        .route("/auth/2fa/setup/verify", post(two_fa_setup_verify))
        .route("/auth/sessions", get(list_sessions))
        .route("/auth/sessions/:session_id", delete(delete_session))
        .route("/auth/logout", post(logout))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ));

    let protected_keys_routes = Router::new()
        .route("/keys/upload", post(upload_keys))
        .route("/keys/upload-prekeys", post(upload_prekeys))
        .route("/keys/me/count", get(get_my_prekey_count))
        .route("/keys/:user_id", get(get_key_bundle))
        .route("/keys/:user_id/fingerprint", get(get_fingerprint))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(keys_state);

    let protected_chat_routes = Router::new()
        .route("/chats", post(create_chat).get(list_chats))
        .route(
            "/chats/:id",
            get(get_chat).patch(update_chat).delete(delete_chat),
        )
        .route("/chats/:id/messages", post(send_message).get(list_messages))
        .route(
            "/chats/:id/messages/:message_id",
            patch(edit_message).delete(delete_message),
        )
        .route("/chats/:id/messages/read", post(mark_messages_read))
        .route(
            "/chats/:id/messages/:message_id/reactions",
            post(add_reaction),
        )
        .route(
            "/chats/:id/messages/:message_id/reactions/:emoji",
            delete(remove_reaction),
        )
        .route("/chats/:id/settings", patch(update_chat_settings))
        .route(
            "/chats/:id/participants",
            get(list_participants).post(add_participant),
        )
        .route(
            "/chats/:id/participants/:user_id",
            delete(remove_participant),
        )
        .route(
            "/chats/:id/participants/:user_id/role",
            patch(update_participant_role),
        )
        .route(
            "/chats/:id/invite-link",
            post(create_invite_link).delete(delete_invite_link),
        )
        .route("/chats/join/:slug", post(join_by_slug))
        .route("/chats/:id/rotate-key", post(rotate_group_key))
        .route("/chats/:id/transfer-ownership", post(transfer_ownership))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(chats_state.clone());

    let protected_notification_routes = Router::new()
        .route("/notifications", get(list_notifications))
        .route(
            "/notifications/read-all",
            patch(mark_all_notifications_read),
        )
        .route("/notifications/read", delete(delete_read_notifications))
        .route(
            "/notifications/:notification_id",
            patch(mark_notification_read),
        )
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(chats_state.clone());

    let protected_attachment_routes = Router::new()
        .route("/attachments/upload-url", post(create_upload_url))
        .route("/attachments/confirm", post(confirm_attachment))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(attachments_state);

    let protected_story_routes = Router::new()
        .route("/stories", post(create_story).get(list_stories))
        .route("/stories/my", get(list_my_stories))
        .route("/stories/:id", delete(delete_story))
        .route("/stories/:id/view", post(view_story))
        .route("/stories/:id/react", post(react_to_story))
        .route("/stories/:id/views", get(get_story_views))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(stories_state);

    let protected_user_routes = Router::new()
        .route("/users/search", get(search_users))
        .route("/users/me/profile", get(get_my_profile))
        .route("/users/:user_id/profile", get(get_user_profile))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(users_state);

    let protected_contact_routes = Router::new()
        .route("/contacts", get(list_contacts).post(create_contact))
        .route(
            "/contacts/:contact_id",
            patch(update_contact).delete(delete_contact),
        )
        .route("/contacts/sync", post(sync_contacts))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(contacts_state);

    let protected_block_routes = Router::new()
        .route("/blocks", get(list_blocked))
        .route("/blocks/:user_id", post(block_user).delete(unblock_user))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(blocks_state);

    let ws_ticket_route = Router::new()
        .route("/ws/ticket", post(crate::handlers::ws::handlers::create_ws_ticket))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(ws_router_state.clone());

    let ws_routes = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(ws_router_state)
        .merge(ws_ticket_route);

    let protected_calls_routes = Router::new()
        .route("/calls/turn-credentials", get(get_turn_credentials))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ))
        .with_state(calls_state);

    let public_routes = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .route("/auth/register", post(register))
        .route("/auth/verify-phone", post(verify_phone))
        .route("/auth/login", post(login))
        .route("/auth/login/verify", post(login_verify))
        .route("/auth/recover", post(recover))
        .route("/auth/recover/verify", post(recover_verify))
        .route("/auth/2fa/verify", post(two_fa_verify))
        .route("/auth/refresh", post(refresh));

    let origins: Vec<HeaderValue> = config
        .server
        .cors_origins
        .split(',')
        .map(|s| s.parse().expect("Invalid CORS origin"))
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    public_routes
        .merge(ws_routes)
        .merge(protected_auth_routes)
        .merge(protected_keys_routes)
        .merge(protected_chat_routes)
        .merge(protected_notification_routes)
        .merge(protected_attachment_routes)
        .merge(protected_story_routes)
        .merge(protected_user_routes)
        .merge(protected_contact_routes)
        .merge(protected_block_routes)
        .merge(protected_calls_routes)
        .layer(cors)
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(logging_middleware))
        .layer(axum::extract::Extension(MetricsExtension(metrics)))
        .with_state(auth_state.clone())
}

async fn health() -> &'static str {
    "OK"
}
