use axum::http::StatusCode;
use axum::{
    Json,
    extract::{ConnectInfo, Extension, Path, State},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use crate::middleware::auth::AuthenticatedUser;
use crate::services::jwt::{JwtService, RefreshSession};
use crate::services::metrics::MetricsExtension;
use crate::services::otp::{OtpService, OtpVerifyResult};
use domain::user::entity::User;
use domain::user::repository::UserRepository;
use domain::user::value_objects::{PhoneNumber, UserId};
use infrastructure::repositories::user::PostgresUserRepository;
use redis::AsyncCommands;
use shared::error::DomainError;

#[derive(Clone)]
pub struct AuthState {
    pub user_repo: Arc<PostgresUserRepository>,
    pub otp_service: Arc<OtpService>,
    pub jwt_service: Arc<JwtService>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RegisterRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct VerifyPhoneRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub device_type: Option<String>,
    pub push_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LoginRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub push_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LoginVerifyRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub device_type: Option<String>,
    pub push_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct RecoverRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct RecoverVerifyRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct TwoFactorSetupRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct TwoFactorChallengeRequest {
    pub temp_token: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub device_type: Option<String>,
    pub push_token: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

#[derive(Debug, serde::Serialize)]
pub struct UserResponse {
    pub id: String,
    pub phone: String,
    pub username: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub device_name: String,
    pub device_type: String,
    pub ip_address: Option<String>,
    pub last_active_at: Option<String>,
    pub is_current: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct SessionsListResponse {
    pub sessions: Vec<SessionResponse>,
}

#[derive(Debug, Clone)]
struct DeviceContext {
    device_id: String,
    device_name: String,
    device_type: String,
    push_token: Option<String>,
}

impl DeviceContext {
    fn from_parts(
        device_id: Option<String>,
        device_name: Option<String>,
        device_type: Option<String>,
        push_token: Option<String>,
    ) -> Self {
        Self {
            device_id: device_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            device_name: device_name.unwrap_or_else(|| "unknown device".to_string()),
            device_type: device_type.unwrap_or_else(|| "web".to_string()),
            push_token,
        }
    }
}

async fn issue_tokens(
    state: &AuthState,
    user: &User,
    device: DeviceContext,
) -> Result<AuthResponse, ApiError> {
    let expires_at = Utc::now() + Duration::seconds(state.jwt_service.refresh_token_ttl() as i64);
    let session_id = state
        .user_repo
        .upsert_session(
            user.id.0,
            &device.device_id,
            &device.device_name,
            &device.device_type,
            device.push_token.as_deref(),
            expires_at,
        )
        .await?;

    let access_token = state
        .jwt_service
        .generate_access_token(&user.id.0, &session_id)?;

    let refresh_session = RefreshSession {
        user_id: user.id.0.to_string(),
        session_id: session_id.to_string(),
        device_id: device.device_id,
        device_name: device.device_name,
        device_type: device.device_type,
        push_token: device.push_token,
    };
    let refresh_token = state.jwt_service.generate_refresh_token(&refresh_session)?;
    state
        .jwt_service
        .store_refresh_token(&refresh_token, &refresh_session)
        .await?;

    Ok(AuthResponse {
        access_token,
        refresh_token,
        expires_in: state.jwt_service.access_token_ttl(),
        user: UserResponse {
            id: user.id.0.to_string(),
            phone: user.phone.as_str().to_string(),
            username: user.username.as_ref().map(|u| u.as_str().to_string()),
        },
    })
}

pub async fn register(
    State(state): State<AuthState>,
    Extension(metrics): Extension<MetricsExtension>,
    ConnectInfo(client_ip): ConnectInfo<SocketAddr>,
    Json(req): Json<RegisterRequest>,
) -> Result<Response, ApiError> {
    // Record attempt
    metrics.0.read().auth_attempts_total.inc();
    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;

    let rate_key = format!("register:{}:{}", client_ip.ip(), req.device_id);
    let allowed = state
        .otp_service
        .check_rate_limit(&rate_key, 5, 3600)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    if !allowed {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(MessageResponse {
                message: "Too many requests".to_string(),
            }),
        )
            .into_response());
    }

    let existing = state.user_repo.find_by_phone(&phone).await?;
    if existing.is_some() {
        return Ok((
            StatusCode::ACCEPTED,
            Json(MessageResponse {
                message: "Código enviado".to_string(),
            }),
        )
            .into_response());
    }

    let code = OtpService::generate();
    state
        .otp_service
        .store_register_otp(phone.as_str(), &code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(MessageResponse {
            message: "Código enviado".to_string(),
        }),
    )
        .into_response())
}

pub async fn verify_phone(
    State(state): State<AuthState>,
    Json(req): Json<VerifyPhoneRequest>,
) -> Result<Response, ApiError> {
    let result = state
        .otp_service
        .verify_register_otp(&req.phone, &req.code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match result {
        OtpVerifyResult::Matched => {}
        OtpVerifyResult::Invalid { remaining } => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: format!("Código inválido. Intentos restantes: {}", remaining),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::Exceeded => {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(MessageResponse {
                    message: "Demasiados intentos fallidos. Por favor solicita un nuevo código."
                        .to_string(),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::NotFound => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: "El código ha expirado. Por favor solicita uno nuevo.".to_string(),
                }),
            )
                .into_response());
        }
    }

    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;

    // Check if user already exists to prevent race condition between OTP verification and creation
    if state.user_repo.find_by_phone(&phone).await?.is_some() {
        return Err(DomainError::AlreadyExists("User already registered".to_string()).into());
    }

    let user = User::new(None, phone, None);
    state.user_repo.create(&user).await?;

    let device = DeviceContext::from_parts(
        req.device_id,
        req.device_name,
        req.device_type,
        req.push_token,
    );
    let auth_response = issue_tokens(&state, &user, device).await?;

    Ok((StatusCode::CREATED, Json(auth_response)).into_response())
}

pub async fn login(
    State(state): State<AuthState>,
    Extension(metrics): Extension<MetricsExtension>,
    ConnectInfo(client_ip): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    // Record attempt
    metrics.0.read().auth_attempts_total.inc();
    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;

    let _user = state
        .user_repo
        .find_by_phone(&phone)
        .await?
        .ok_or_else(|| DomainError::Unauthorized("Credenciales inválidas".to_string()))?;

    let rate_key = format!("login:{}:{}", client_ip.ip(), req.device_id);
    let allowed = state
        .otp_service
        .check_rate_limit(&rate_key, 10, 3600)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    if !allowed {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(MessageResponse {
                message: "Too many requests".to_string(),
            }),
        )
            .into_response());
    }

    let code = OtpService::generate();
    state
        .otp_service
        .store_login_otp(phone.as_str(), &code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(MessageResponse {
            message: "Código enviado".to_string(),
        }),
    )
        .into_response())
}

pub async fn login_verify(
    State(state): State<AuthState>,
    Json(req): Json<LoginVerifyRequest>,
) -> Result<Response, ApiError> {
    let result = state
        .otp_service
        .verify_login_otp(&req.phone, &req.code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match result {
        OtpVerifyResult::Matched => {}
        OtpVerifyResult::Invalid { remaining } => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: format!("Código inválido. Intentos restantes: {}", remaining),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::Exceeded => {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(MessageResponse {
                    message: "Demasiados intentos fallidos. Por favor solicita un nuevo código."
                        .to_string(),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::NotFound => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: "El código ha expirado. Por favor solicita uno nuevo.".to_string(),
                }),
            )
                .into_response());
        }
    }

    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;

    let user = state
        .user_repo
        .find_by_phone(&phone)
        .await?
        .ok_or_else(|| DomainError::Unauthorized("Usuario no encontrado".to_string()))?;

    if user.two_fa_enabled {
        let two_fa_code = OtpService::generate();
        state
            .otp_service
            .store_two_fa_login_otp(&user.id.0.to_string(), &two_fa_code)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        let temp_token = state.jwt_service.generate_temp_token(&user.id.0)?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "two_fa_required": true,
                "temp_token": temp_token
            })),
        )
            .into_response());
    }

    let device = DeviceContext::from_parts(
        req.device_id,
        req.device_name,
        req.device_type,
        req.push_token,
    );
    let auth_response = issue_tokens(&state, &user, device).await?;

    Ok((StatusCode::OK, Json(auth_response)).into_response())
}

pub async fn refresh(
    State(state): State<AuthState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Response, ApiError> {
    let refresh_session = state
        .jwt_service
        .validate_refresh_token(&req.refresh_token)
        .await
        .map_err(|e| DomainError::Unauthorized(e.to_string()))?;

    let user_id = Uuid::parse_str(&refresh_session.user_id)
        .map_err(|e| DomainError::Internal(e.to_string()))?;
    let user = state
        .user_repo
        .find_by_id(&UserId(user_id))
        .await?
        .ok_or_else(|| DomainError::Unauthorized("Usuario no encontrado".to_string()))?;
    let device = DeviceContext::from_parts(
        Some(refresh_session.device_id),
        Some(refresh_session.device_name),
        Some(refresh_session.device_type),
        refresh_session.push_token,
    );
    let auth_response = issue_tokens(&state, &user, device).await?;

    Ok((StatusCode::OK, Json(auth_response)).into_response())
}

pub async fn recover(
    State(state): State<AuthState>,
    Json(req): Json<RecoverRequest>,
) -> Result<Response, ApiError> {
    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;
    if state.user_repo.find_by_phone(&phone).await?.is_some() {
        let code = OtpService::generate();
        state
            .otp_service
            .store_recover_otp(phone.as_str(), &code)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(MessageResponse {
            message: "Si la cuenta existe, enviamos un codigo de recuperacion".to_string(),
        }),
    )
        .into_response())
}

pub async fn recover_verify(
    State(state): State<AuthState>,
    Json(req): Json<RecoverVerifyRequest>,
) -> Result<Response, ApiError> {
    let result = state
        .otp_service
        .verify_recover_otp(&req.phone, &req.code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match result {
        OtpVerifyResult::Matched => {}
        OtpVerifyResult::Invalid { remaining } => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: format!("Código inválido. Intentos restantes: {}", remaining),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::Exceeded => {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(MessageResponse {
                    message: "Demasiados intentos fallidos. Por favor solicita un nuevo código."
                        .to_string(),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::NotFound => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: "El código ha expirado. Por favor solicita uno nuevo.".to_string(),
                }),
            )
                .into_response());
        }
    }

    let phone =
        PhoneNumber::new(req.phone.clone()).map_err(|e| DomainError::Validation(e.to_string()))?;
    let user = state
        .user_repo
        .find_by_phone(&phone)
        .await?
        .ok_or_else(|| DomainError::Unauthorized("Usuario no encontrado".to_string()))?;
    let recover_token = state.jwt_service.generate_temp_token(&user.id.0)?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "recover_token": recover_token })),
    )
        .into_response())
}

pub async fn two_fa_setup(
    State(state): State<AuthState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    let code = OtpService::generate();
    state
        .otp_service
        .store_two_fa_setup_otp(&auth.user_id.to_string(), &code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "2FA setup code generated",
            "code": code
        })),
    )
        .into_response())
}

pub async fn two_fa_setup_verify(
    State(state): State<AuthState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<TwoFactorSetupRequest>,
) -> Result<Response, ApiError> {
    let result = state
        .otp_service
        .verify_two_fa_setup_otp(&auth.user_id.to_string(), &req.code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match result {
        OtpVerifyResult::Matched => {}
        OtpVerifyResult::Invalid { remaining } => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: format!(
                        "Código de configuración inválido. Intentos restantes: {}",
                        remaining
                    ),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::Exceeded => {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(MessageResponse {
                    message: "Demasiados intentos fallidos. Por favor inicia la configuración de 2FA nuevamente."
                        .to_string(),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::NotFound => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: "El código ha expirado. Por favor inicia la configuración de 2FA nuevamente.".to_string(),
                }),
            )
                .into_response());
        }
    }

    let mut user = state
        .user_repo
        .find_by_id(&UserId(auth.user_id))
        .await?
        .ok_or_else(|| DomainError::NotFound("Usuario no encontrado".to_string()))?;
    user.two_fa_enabled = true;
    state.user_repo.update(&user).await?;

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            message: "2FA habilitado".to_string(),
        }),
    )
        .into_response())
}

pub async fn two_fa_verify(
    State(state): State<AuthState>,
    Json(req): Json<TwoFactorChallengeRequest>,
) -> Result<Response, ApiError> {
    let claims = state
        .jwt_service
        .validate_access_token(&req.temp_token)
        .map_err(|e| DomainError::Unauthorized(e.to_string()))?;
    if claims.sid != "temp" {
        return Err(DomainError::Unauthorized("Token temporal invalido".to_string()).into());
    }

    let user_id =
        Uuid::parse_str(&claims.sub).map_err(|e| DomainError::Unauthorized(e.to_string()))?;
    let result = state
        .otp_service
        .verify_two_fa_login_otp(&user_id.to_string(), &req.code)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match result {
        OtpVerifyResult::Matched => {}
        OtpVerifyResult::Invalid { remaining } => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: format!("Código 2FA inválido. Intentos restantes: {}", remaining),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::Exceeded => {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(MessageResponse {
                    message: "Demasiados intentos fallidos. Por favor solicita un nuevo código."
                        .to_string(),
                }),
            )
                .into_response());
        }
        OtpVerifyResult::NotFound => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(MessageResponse {
                    message: "El código ha expirado. Por favor solicita uno nuevo.".to_string(),
                }),
            )
                .into_response());
        }
    }

    let user = state
        .user_repo
        .find_by_id(&UserId(user_id))
        .await?
        .ok_or_else(|| DomainError::Unauthorized("Usuario no encontrado".to_string()))?;
    let device = DeviceContext::from_parts(
        req.device_id,
        req.device_name,
        req.device_type,
        req.push_token,
    );
    let auth_response = issue_tokens(&state, &user, device).await?;

    Ok((StatusCode::OK, Json(auth_response)).into_response())
}

pub async fn list_sessions(
    State(state): State<AuthState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    let sessions = state.user_repo.list_sessions(auth.user_id).await?;
    let response = SessionsListResponse {
        sessions: sessions
            .into_iter()
            .map(|s| SessionResponse {
                id: s.id.to_string(),
                device_name: s.device_name,
                device_type: s.device_type,
                ip_address: s.ip_address,
                last_active_at: s.last_active_at.map(|ts| ts.to_rfc3339()),
                is_current: s.id == auth.session_id,
            })
            .collect(),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn delete_session(
    State(state): State<AuthState>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(session_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let deleted = state
        .user_repo
        .delete_session(auth.user_id, session_id)
        .await?;
    if !deleted {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(MessageResponse {
                message: "Sesion no encontrada".to_string(),
            }),
        )
            .into_response());
    }

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            message: "Sesion eliminada".to_string(),
        }),
    )
        .into_response())
}

pub async fn logout(
    State(state): State<AuthState>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> Result<Response, ApiError> {
    state
        .user_repo
        .delete_session(auth.user_id, auth.session_id)
        .await?;

    let mut redis = state.otp_service.redis.clone();
    let session_key = format!("session:{}", auth.session_id);
    let _: () = redis
        .del(session_key)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    let refresh_key = format!("refresh:{}", auth.session_id);
    let _: () = redis
        .del(refresh_key)
        .await
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(MessageResponse {
            message: "Logout exitoso".to_string(),
        }),
    )
        .into_response())
}

pub fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrNumberVisitor;

    impl<'de> serde::de::Visitor<'de> for StringOrNumberVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or a number")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }
    }

    deserializer.deserialize_any(StringOrNumberVisitor)
}

#[cfg(test)]
mod auth_dto_tests {
    use super::*;

    #[test]
    fn test_flexible_deserialization() {
        // Test with string values
        let json_str = r#"
        {
            "phone": "+573001234567",
            "code": "123456",
            "device_id": "test-device",
            "device_name": "Test Device",
            "device_type": "iOS"
        }
        "#;
        let req1: LoginVerifyRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req1.phone, "+573001234567");
        assert_eq!(req1.code, "123456");

        // Test with number values
        let json_num = r#"
        {
            "phone": 573001234567,
            "code": 123456,
            "device_id": "test-device",
            "device_name": "Test Device",
            "device_type": "iOS"
        }
        "#;
        let req2: LoginVerifyRequest = serde_json::from_str(json_num).unwrap();
        assert_eq!(req2.phone, "573001234567");
        assert_eq!(req2.code, "123456");
    }
}
