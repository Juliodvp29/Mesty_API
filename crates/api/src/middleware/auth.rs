use axum::{
    Json,
    extract::Request,
    extract::State,
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::jwt::JwtService;

#[derive(Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    pub session_id: Uuid,
}

pub async fn auth_middleware(
    State(state): State<Arc<AuthMiddlewareState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(auth) = auth_header else {
        return unauthorized_response("Missing bearer token");
    };
    if !auth.starts_with("Bearer ") {
        return unauthorized_response("Invalid authorization scheme");
    }

    let token = &auth[7..];
    let claims = match state.jwt_service.validate_access_token(token) {
        Ok(claims) => claims,
        Err(e) => {
            tracing::error!("Auth middleware error: {:?}", e);
            return unauthorized_response("Invalid or expired token");
        }
    };

    let user_id = match Uuid::parse_str(&claims.sub) {
        Ok(value) => value,
        Err(_) => return unauthorized_response("Malformed token subject"),
    };

    // Rechazar explícitamente tokens temporales de 2FA / recuperación en rutas protegidas.
    // Estos tokens tienen sid = TEMP_SESSION_ID y solo son válidos para /auth/2fa/verify
    // y /auth/recover/verify, que los consumen directamente desde el body sin pasar por este middleware.
    if claims.sid == crate::services::jwt::TEMP_SESSION_ID {
        tracing::warn!(user_id = %user_id, "Rejected attempt to use a temporary 2FA token on a protected route");
        return unauthorized_response("Temporary 2FA token cannot be used for this endpoint");
    }

    let session_id = match Uuid::parse_str(&claims.sid) {
        Ok(value) => value,
        Err(_) => return unauthorized_response("Malformed token session"),
    };

    let session_valid = validate_session(&state, user_id, session_id).await;
    if !session_valid {
        return unauthorized_response("Session revoked or expired");
    }

    req.extensions_mut().insert(AuthenticatedUser {
        user_id,
        session_id,
    });

    next.run(req).await
}

async fn validate_session(state: &AuthMiddlewareState, user_id: Uuid, session_id: Uuid) -> bool {
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", session_id);

    let cached: Option<String> = redis.get(&session_key).await.ok().flatten();
    if cached.is_some() {
        return true;
    }

    match state.user_repo.is_session_valid(user_id, session_id).await {
        Ok(true) => {
            let ttl = state.jwt_service.access_token_ttl();
            let _: Option<()> = redis.set_ex(session_key, "active", ttl).await.ok();
            true
        }
        _ => false,
    }
}

pub struct AuthMiddlewareState {
    pub jwt_service: Arc<JwtService>,
    pub redis: redis::aio::ConnectionManager,
    pub user_repo: Arc<infrastructure::repositories::user::PostgresUserRepository>,
}

fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}
