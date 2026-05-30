use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::services::error::ServiceError;

/// Valor reservado para el campo `sid` de tokens de autenticación de 2FA / recuperación.
/// Estos tokens son temporales y NO deben ser aceptados en rutas protegidas.
pub const TEMP_SESSION_ID: &str = "temp";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: String,
    pub sid: String,
    pub exp: i64,
    pub iat: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshData {
    pub sub: String,
    pub sid: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub push_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshSession {
    pub user_id: String,
    pub session_id: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub push_token: Option<String>,
}

#[derive(Clone)]
pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    access_ttl: u64,
    refresh_ttl: u64,
    redis: Option<ConnectionManager>,
}

impl JwtService {
    pub fn new(
        private_key_pem: String,
        public_key_pem: String,
        access_ttl: u64,
        refresh_ttl: u64,
        redis: Option<ConnectionManager>,
    ) -> Self {
        let encoding_key =
            EncodingKey::from_ed_pem(private_key_pem.as_bytes()).expect("Invalid JWT private key");
        let decoding_key =
            DecodingKey::from_ed_pem(public_key_pem.as_bytes()).expect("Invalid JWT public key");

        Self {
            encoding_key,
            decoding_key,
            access_ttl,
            refresh_ttl,
            redis,
        }
    }

    pub fn generate_access_token(
        &self,
        user_id: &Uuid,
        session_id: &Uuid,
    ) -> Result<String, ServiceError> {
        let now = chrono::Utc::now().timestamp();
        let claims = AccessClaims {
            sub: user_id.to_string(),
            sid: session_id.to_string(),
            exp: now + self.access_ttl as i64,
            iat: now,
        };

        let header = Header::new(Algorithm::EdDSA);

        let token = encode(&header, &claims, &self.encoding_key)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(token)
    }

    pub fn generate_temp_token(&self, user_id: &Uuid) -> Result<String, ServiceError> {
        let now = chrono::Utc::now().timestamp();
        let claims = AccessClaims {
            sub: user_id.to_string(),
            sid: TEMP_SESSION_ID.to_string(),
            exp: now + 300,
            iat: now,
        };

        let header = Header::new(Algorithm::EdDSA);

        let token = encode(&header, &claims, &self.encoding_key)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(token)
    }

    pub fn validate_access_token(&self, token: &str) -> Result<AccessClaims, ServiceError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = true;
        let claims = decode::<AccessClaims>(token, &self.decoding_key, &validation)
            .map_err(|e| ServiceError::Unauthorized(e.to_string()))?
            .claims;

        let now = chrono::Utc::now().timestamp();
        if claims.exp < now {
            return Err(ServiceError::Unauthorized("Token expired".to_string()));
        }

        Ok(claims)
    }

    pub fn generate_refresh_token(
        &self,
        _session: &RefreshSession,
    ) -> Result<String, ServiceError> {
        let token = Uuid::new_v4().to_string();
        Ok(token)
    }

    pub async fn store_refresh_token(
        &self,
        token: &str,
        session: &RefreshSession,
    ) -> Result<(), ServiceError> {
        let redis = self
            .redis
            .as_ref()
            .ok_or_else(|| ServiceError::Internal("Redis not configured".to_string()))?;
        let mut con = redis.clone();

        let hash = Self::sha256_hash(token);
        let key = format!("refresh:{}", hash);
        let json =
            serde_json::to_string(session).map_err(|e| ServiceError::Internal(e.to_string()))?;

        let _: () = con
            .set_ex(key, json, self.refresh_ttl)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(())
    }

    pub async fn validate_refresh_token(
        &self,
        token: &str,
    ) -> Result<RefreshSession, ServiceError> {
        let redis = self
            .redis
            .as_ref()
            .ok_or_else(|| ServiceError::Internal("Redis not configured".to_string()))?;
        let mut con = redis.clone();

        let hash = Self::sha256_hash(token);
        let key = format!("refresh:{}", hash);

        let session: Option<String> = con
            .get(&key)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let session = session
            .ok_or_else(|| ServiceError::Unauthorized("Invalid refresh token".to_string()))?;

        let session: RefreshSession =
            serde_json::from_str(&session).map_err(|e| ServiceError::Internal(e.to_string()))?;

        let _: usize = con
            .del(&key)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(session)
    }

    pub async fn rotate_refresh_token(
        &self,
        _old_token: &str,
        session: &RefreshSession,
    ) -> Result<String, ServiceError> {
        let new_token = Uuid::new_v4().to_string();

        self.store_refresh_token(&new_token, session).await?;

        Ok(new_token)
    }

    fn sha256_hash(input: &str) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    pub fn access_token_ttl(&self) -> u64 {
        self.access_ttl
    }

    pub fn refresh_token_ttl(&self) -> u64 {
        self.refresh_ttl
    }
}
