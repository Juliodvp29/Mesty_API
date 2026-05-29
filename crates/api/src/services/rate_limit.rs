use redis::aio::ConnectionManager;

/// Checks if a request is within the allowed rate limit using a sliding/fixed window in Redis.
///
/// # Arguments
/// * `redis` - A mutable reference to the Redis ConnectionManager.
/// * `key` - The unique identifier for the rate limit (e.g., "msg:user_id").
/// * `limit` - The maximum number of allowed requests in the time window.
/// * `window` - The duration of the time window in seconds.
///
/// # Returns
/// * `Ok(true)` if the request is allowed.
/// * `Ok(false)` if the rate limit has been exceeded.
pub async fn check_rate_limit(
    redis: &mut ConnectionManager,
    key: &str,
    limit: u64,
    window: u64,
) -> Result<bool, redis::RedisError> {
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
        .invoke_async(redis)
        .await?;

    Ok(count <= limit)
}
