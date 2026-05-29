use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyPhoneRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub push_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginVerifyRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub phone: String,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub device_name: String,
    pub device_type: String,
    pub ip_address: Option<String>,
    pub last_active_at: Option<String>,
    pub is_current: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionsListResponse {
    pub sessions: Vec<SessionResponse>,
}

#[derive(Debug, Deserialize)]
pub struct TwoFactorSetupRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct TwoFactorChallengeRequest {
    pub temp_token: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct RecoverRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
}

#[derive(Debug, Deserialize)]
pub struct RecoverVerifyRequest {
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub phone: String,
    #[serde(deserialize_with = "deserialize_string_or_number")]
    pub code: String,
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
            "code": "123456"
        }
        "#;
        let req1: LoginVerifyRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req1.phone, "+573001234567");
        assert_eq!(req1.code, "123456");

        // Test with number values
        let json_num = r#"
        {
            "phone": 573001234567,
            "code": 123456
        }
        "#;
        let req2: LoginVerifyRequest = serde_json::from_str(json_num).unwrap();
        assert_eq!(req2.phone, "573001234567");
        assert_eq!(req2.code, "123456");
    }
}
