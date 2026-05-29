use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AppEnv {
    Development,
    Staging,
    Production,
}

impl AppEnv {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppEnv::Development => "development",
            AppEnv::Staging => "staging",
            AppEnv::Production => "production",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub app_env: AppEnv,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub jwt: JwtConfig,
    pub s3: S3Config,
    pub smtp: SmtpConfig,
    pub sms: SmsConfig,
    pub push: PushConfig,
    #[serde(default)]
    pub turn: TurnConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub cors_origins: String,
    pub rate_limit_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JwtConfig {
    pub private_key: String,
    pub public_key: String,
    pub access_ttl_seconds: u64,
    pub refresh_ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub from: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmsConfig {
    pub provider: String,
    pub api_key: String,
    pub sender_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushConfig {
    pub fcm_api_key: Option<String>,
    pub apns_key_path: Option<String>,
    pub apns_key_id: Option<String>,
    pub apns_team_id: Option<String>,
    pub apns_bundle_id: String,
}

/// STUN/TURN server configuration for WebRTC.
///
/// Compatible with coturn's `use-auth-secret` mode:
///   username = "<expires_epoch>:<user_id>"
///   password = base64(HMAC-SHA1(secret, username))
///
/// Set via environment variables (double-underscore separator):
///   TURN__SECRET, TURN__STUN_URLS, TURN__TURN_URLS, TURN__TTL_SECONDS
///
/// `stun_urls` and `turn_urls` are comma-separated lists, e.g.:
///   TURN__STUN_URLS="stun:stun.l.google.com:19302,stun:stun1.l.google.com:19302"
///   TURN__TURN_URLS="turn:turn.example.com:3478,turns:turn.example.com:5349"
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TurnConfig {
    /// HMAC-SHA1 secret used to sign time-limited credentials.
    /// If None, only public STUN servers (no credentials) are returned.
    pub secret: Option<String>,
    /// Optional override for credential TTL (seconds). Defaults to 3600.
    pub ttl_seconds: Option<u64>,
    /// Comma-separated STUN server URLs. Deserialized from env as Vec<String>.
    #[serde(default = "default_stun_urls")]
    pub stun_urls: Vec<String>,
    /// Comma-separated TURN server URLs.
    #[serde(default)]
    pub turn_urls: Vec<String>,
}

fn default_stun_urls() -> Vec<String> {
    vec![
        "stun:stun.l.google.com:19302".to_string(),
        "stun:stun1.l.google.com:19302".to_string(),
    ]
}

impl Config {
    pub fn load() -> Result<Self, config::ConfigError> {
        let settings = config::Config::builder()
            .add_source(config::Environment::default().separator("__"))
            .build()?;

        let mut config: Self = settings.try_deserialize()?;

        // Fix newlines in JWT keys if they come from .env with literal \n
        config.jwt.private_key = config.jwt.private_key.replace("\\n", "\n");
        config.jwt.public_key = config.jwt.public_key.replace("\\n", "\n");

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_load() {
        let _ = dotenvy::dotenv();
        // In CI or locally with .env, this should load without errors.
        // It will fail if any required variables are missing.
        let config = Config::load();
        assert!(config.is_ok(), "Config load failed: {:?}", config.err());
    }
}
