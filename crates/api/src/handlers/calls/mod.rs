use axum::extract::Extension;
use axum::{Json, extract::State};
use serde::Serialize;
use std::sync::Arc;

use crate::middleware::auth::AuthenticatedUser;
use shared::config::TurnConfig;

/// State for the calls REST handlers.
#[derive(Clone)]
pub struct CallsState {
    pub turn_config: Arc<TurnConfig>,
}

/// Response returned by `GET /calls/turn-credentials`.
/// The client uses these to configure the RTCPeerConnection's iceServers.
#[derive(Serialize)]
pub struct TurnCredentialsResponse {
    /// List of ICE servers ready to be passed to `RTCPeerConnection`.
    pub ice_servers: Vec<IceServer>,
    /// Unix timestamp (seconds) when these credentials expire.
    pub expires_at: i64,
}

#[derive(Serialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// `GET /api/v1/calls/turn-credentials`
///
/// Returns short-lived TURN credentials so the client can establish
/// peer-to-peer WebRTC connections even through strict NATs/firewalls.
///
/// Credential generation strategy:
///  - username = `<ttl_epoch>:<user_id>`  (time-limited)
///  - password = HMAC-SHA1(secret, username)  — compatible with coturn's
///    `use-auth-secret` mode and with Twilio Network Traversal Service.
///
/// The credentials expire in `turn.ttl_seconds` (default 3600s = 1h).
pub async fn get_turn_credentials(
    Extension(auth_user): Extension<AuthenticatedUser>,
    State(state): State<CallsState>,
) -> Json<TurnCredentialsResponse> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    let user_id = auth_user.user_id;
    let config = &state.turn_config;
    let ttl = config.ttl_seconds.unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + ttl as i64;

    // username = "<expiry_epoch>:<user_id>"
    let username = format!("{}:{}", expires_at, user_id);

    // password = base64(HMAC-SHA1(turn_secret, username))
    let credential = if let Some(ref secret) = config.secret {
        type HmacSha1 = Hmac<Sha1>;
        let mut mac =
            HmacSha1::new_from_slice(secret.as_bytes()).expect("HMAC can work with any key size");
        mac.update(username.as_bytes());
        let result = mac.finalize().into_bytes();
        Some(STANDARD.encode(result))
    } else {
        None
    };

    let mut ice_servers: Vec<IceServer> = Vec::new();

    // Public STUN servers (no credentials needed)
    if !config.stun_urls.is_empty() {
        ice_servers.push(IceServer {
            urls: config.stun_urls.clone(),
            username: None,
            credential: None,
        });
    }

    // TURN servers (time-limited credentials)
    if !config.turn_urls.is_empty() {
        ice_servers.push(IceServer {
            urls: config.turn_urls.clone(),
            username: Some(username),
            credential,
        });
    }

    Json(TurnCredentialsResponse {
        ice_servers,
        expires_at,
    })
}
