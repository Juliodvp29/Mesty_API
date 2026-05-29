use redis::aio::ConnectionManager;

/// Result of an OTP verification attempt.
///
/// - [`OtpVerifyResult::Matched`]           – Code was correct; OTP has been consumed.
/// - [`OtpVerifyResult::Invalid`]           – Code was wrong; contains remaining attempts.
/// - [`OtpVerifyResult::Exceeded`]          – Attempt limit reached; OTP has been invalidated.
/// - [`OtpVerifyResult::NotFound`]          – OTP key does not exist (expired or never issued).
#[derive(Debug, PartialEq, Eq)]
pub enum OtpVerifyResult {
    Matched,
    Invalid { remaining: u64 },
    Exceeded,
    NotFound,
}

#[derive(Clone)]
pub struct OtpService {
    pub redis: ConnectionManager,
    otp_ttl: u64,
}

impl OtpService {
    pub fn new(redis: ConnectionManager, otp_ttl: u64) -> Self {
        Self { redis, otp_ttl }
    }

    pub fn generate() -> String {
        let code = rand::random::<u32>() % 1_000_000;
        format!("{:06}", code)
    }

    // -------------------------------------------------------------------------
    // OTP storage helpers
    // -------------------------------------------------------------------------

    pub async fn store_register_otp(
        &self,
        phone: &str,
        code: &str,
    ) -> Result<(), redis::RedisError> {
        let mut con = self.redis.clone();
        let key = format!("otp:register:{}", phone);
        let _: () = redis::AsyncCommands::set_ex(&mut con, key, code, self.otp_ttl).await?;
        Ok(())
    }

    pub async fn store_login_otp(&self, phone: &str, code: &str) -> Result<(), redis::RedisError> {
        let mut con = self.redis.clone();
        let key = format!("otp:login:{}", phone);
        let _: () = redis::AsyncCommands::set_ex(&mut con, key, code, self.otp_ttl).await?;
        Ok(())
    }

    pub async fn store_recover_otp(
        &self,
        phone: &str,
        code: &str,
    ) -> Result<(), redis::RedisError> {
        let mut con = self.redis.clone();
        let key = format!("otp:recover:{}", phone);
        let _: () = redis::AsyncCommands::set_ex(&mut con, key, code, self.otp_ttl).await?;
        Ok(())
    }

    pub async fn store_two_fa_setup_otp(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<(), redis::RedisError> {
        let mut con = self.redis.clone();
        let key = format!("otp:2fa:setup:{}", user_id);
        let _: () = redis::AsyncCommands::set_ex(&mut con, key, code, self.otp_ttl).await?;
        Ok(())
    }

    pub async fn store_two_fa_login_otp(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<(), redis::RedisError> {
        let mut con = self.redis.clone();
        let key = format!("otp:2fa:login:{}", user_id);
        let _: () = redis::AsyncCommands::set_ex(&mut con, key, code, self.otp_ttl).await?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Core atomic verification with brute-force protection
    // -------------------------------------------------------------------------

    /// Atomically verifies an OTP code and tracks failed attempts using a single
    /// Redis Lua script to prevent race conditions.
    ///
    /// The attempt counter key is `{otp_key}:attempts`. On the first failed attempt
    /// it inherits the remaining TTL of the OTP key so both are cleaned up together.
    ///
    /// Lua return array `[status, remaining]`:
    ///  * `[ 1, 0]` → match; OTP and counter keys deleted.
    ///  * `[ 0, N]` → wrong code; N attempts remaining before lockout.
    ///  * `[-1, 0]` → OTP key not found / already expired.
    ///  * `[-2, 0]` → attempt limit reached; OTP key invalidated.
    async fn verify_otp_with_limit(
        &self,
        otp_key: &str,
        submitted_code: &str,
        max_attempts: u64,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let mut con = self.redis.clone();
        let attempts_key = format!("{}:attempts", otp_key);

        let lua_script = r#"
            local stored = redis.call('GET', KEYS[1])
            if not stored then
                return {-1, 0}
            end

            if stored == ARGV[1] then
                redis.call('DEL', KEYS[1])
                redis.call('DEL', KEYS[2])
                return {1, 0}
            end

            local attempts = redis.call('INCR', KEYS[2])
            if attempts == 1 then
                local ttl = redis.call('TTL', KEYS[1])
                if ttl > 0 then
                    redis.call('EXPIRE', KEYS[2], ttl)
                end
            end

            local max = tonumber(ARGV[2])
            if attempts >= max then
                redis.call('DEL', KEYS[1])
                redis.call('DEL', KEYS[2])
                return {-2, 0}
            end

            return {0, max - attempts}
        "#;

        let result: Vec<i64> = redis::Script::new(lua_script)
            .key(otp_key)
            .key(&attempts_key)
            .arg(submitted_code)
            .arg(max_attempts)
            .invoke_async(&mut con)
            .await?;

        let status = result.first().copied().unwrap_or(-1);
        let remaining = result.get(1).copied().unwrap_or(0).max(0) as u64;

        Ok(match status {
            1 => OtpVerifyResult::Matched,
            0 => OtpVerifyResult::Invalid { remaining },
            -2 => OtpVerifyResult::Exceeded,
            _ => OtpVerifyResult::NotFound,
        })
    }

    // -------------------------------------------------------------------------
    // Public verification methods (all delegate to verify_otp_with_limit)
    // -------------------------------------------------------------------------

    pub async fn verify_register_otp(
        &self,
        phone: &str,
        code: &str,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let key = format!("otp:register:{}", phone);
        self.verify_otp_with_limit(&key, code, 3).await
    }

    pub async fn verify_login_otp(
        &self,
        phone: &str,
        code: &str,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let key = format!("otp:login:{}", phone);
        self.verify_otp_with_limit(&key, code, 3).await
    }

    pub async fn verify_recover_otp(
        &self,
        phone: &str,
        code: &str,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let key = format!("otp:recover:{}", phone);
        self.verify_otp_with_limit(&key, code, 3).await
    }

    pub async fn verify_two_fa_setup_otp(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let key = format!("otp:2fa:setup:{}", user_id);
        self.verify_otp_with_limit(&key, code, 3).await
    }

    pub async fn verify_two_fa_login_otp(
        &self,
        user_id: &str,
        code: &str,
    ) -> Result<OtpVerifyResult, redis::RedisError> {
        let key = format!("otp:2fa:login:{}", user_id);
        self.verify_otp_with_limit(&key, code, 3).await
    }

    // -------------------------------------------------------------------------
    // Rate limiting (for send endpoints — /login, /register, etc.)
    // -------------------------------------------------------------------------

    pub async fn check_rate_limit(
        &self,
        key: &str,
        limit: u64,
        window: u64,
    ) -> Result<bool, redis::RedisError> {
        let mut con = self.redis.clone();
        let rate_key = format!("rate:{}", key);

        let lua_script = r#"
            local count = redis.call('INCR', KEYS[1])
            if count == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return count
        "#;

        let count: u64 = redis::Script::new(lua_script)
            .key(&rate_key)
            .arg(window)
            .invoke_async(&mut con)
            .await?;

        Ok(count <= limit)
    }
}
