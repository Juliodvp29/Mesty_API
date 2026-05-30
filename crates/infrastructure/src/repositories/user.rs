use chrono::{DateTime, Utc};
use domain::user::entity::User;
use domain::user::repository::UserRepository;
use domain::user::value_objects::{Email, PhoneNumber, UserId, Username};
use shared::error::{DomainError, DomainResult};
use sqlx::PgPool;
use uuid::Uuid;

use redis::aio::ConnectionManager;

pub struct PostgresUserRepository {
    pool: PgPool,
    redis: Option<ConnectionManager>,
}

#[derive(Debug, Clone)]
pub struct UserSessionRecord {
    pub id: Uuid,
    pub device_name: String,
    pub device_type: String,
    pub ip_address: Option<String>,
    pub last_active_at: Option<DateTime<Utc>>,
}

impl PostgresUserRepository {
    pub fn new(pool: PgPool, redis: Option<ConnectionManager>) -> Self {
        Self { pool, redis }
    }

    pub async fn upsert_session(
        &self,
        user_id: Uuid,
        device_id: &str,
        device_name: &str,
        device_type: &str,
        push_token: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> DomainResult<Uuid> {
        let session_id = Uuid::new_v4();

        let record = sqlx::query_as::<_, (Uuid,)>(
            r#"
            INSERT INTO user_sessions (id, user_id, device_id, device_name, device_type, push_token, expires_at)
            VALUES ($1, $2, $3, $4, $5::device_type, $6, $7)
            ON CONFLICT (user_id, device_id) DO UPDATE
            SET
                id = EXCLUDED.id,
                device_name = EXCLUDED.device_name,
                device_type = EXCLUDED.device_type,
                push_token = EXCLUDED.push_token,
                expires_at = EXCLUDED.expires_at,
                last_active_at = NOW()
            RETURNING id
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(device_id)
        .bind(device_name)
        .bind(device_type)
        .bind(push_token)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(record.0)
    }

    pub async fn list_sessions(&self, user_id: Uuid) -> DomainResult<Vec<UserSessionRecord>> {
        let records =
            sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<DateTime<Utc>>)>(
                r#"
            SELECT
                id,
                device_name,
                device_type::text,
                ip_address::text,
                last_active_at
            FROM user_sessions
            WHERE user_id = $1
            ORDER BY last_active_at DESC
            "#,
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;

        Ok(records
            .into_iter()
            .map(
                |(id, device_name, device_type, ip_address, last_active_at)| UserSessionRecord {
                    id,
                    device_name,
                    device_type,
                    ip_address,
                    last_active_at,
                },
            )
            .collect())
    }

    pub async fn delete_session(&self, user_id: Uuid, session_id: Uuid) -> DomainResult<bool> {
        let result = sqlx::query("DELETE FROM user_sessions WHERE id = $1 AND user_id = $2")
            .bind(session_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn is_session_valid(&self, user_id: Uuid, session_id: Uuid) -> DomainResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar(
            r#"
            SELECT 1 FROM user_sessions 
            WHERE id = $1 AND user_id = $2 AND expires_at > NOW()
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(exists.is_some())
    }
}

#[derive(sqlx::FromRow)]
struct UserRecord {
    id: Uuid,
    username: Option<String>,
    phone: String,
    phone_hash: String,
    email: Option<String>,
    status_text: Option<String>,
    two_fa_enabled: bool,
    two_fa_secret: Option<String>,
    is_active: bool,
    last_seen_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

impl UserRepository for PostgresUserRepository {
    async fn create(&self, user: &User) -> DomainResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO users (id, username, phone, phone_hash, email, status_text, two_fa_enabled, two_fa_secret, is_active, last_seen_at, created_at, updated_at, deleted_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
            user.id.0,
            user.username.as_ref().map(|u| u.as_str()),
            user.phone.as_str(),
            user.phone_hash,
            user.email.as_ref().map(|e| e.as_str()),
            user.status_text,
            user.two_fa_enabled,
            user.two_fa_secret,
            user.is_active,
            user.last_seen_at,
            user.created_at,
            user.updated_at,
            user.deleted_at
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if let Some(mut redis) = self.redis.clone() {
            use redis::AsyncCommands;
            let _: () = redis
                .sadd("phone_hashes", &user.phone_hash)
                .await
                .map_err(|e| DomainError::Internal(e.to_string()))?;
        }

        Ok(())
    }

    async fn find_by_id(&self, id: &UserId) -> DomainResult<Option<User>> {
        let record = sqlx::query_as!(
            UserRecord,
            r#"
            SELECT 
                id AS "id!", 
                username, 
                phone AS "phone!", 
                phone_hash AS "phone_hash!", 
                email, 
                status_text, 
                two_fa_enabled AS "two_fa_enabled!", 
                two_fa_secret, 
                is_active AS "is_active!", 
                last_seen_at, 
                created_at AS "created_at!", 
                updated_at AS "updated_at!", 
                deleted_at 
            FROM users 
            WHERE id = $1 AND deleted_at IS NULL
            "#,
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_user).transpose()
    }

    async fn find_by_phone(&self, phone: &PhoneNumber) -> DomainResult<Option<User>> {
        let record = sqlx::query_as!(
            UserRecord,
            r#"
            SELECT 
                id AS "id!", 
                username, 
                phone AS "phone!", 
                phone_hash AS "phone_hash!", 
                email, 
                status_text, 
                two_fa_enabled AS "two_fa_enabled!", 
                two_fa_secret, 
                is_active AS "is_active!", 
                last_seen_at, 
                created_at AS "created_at!", 
                updated_at AS "updated_at!", 
                deleted_at 
            FROM users 
            WHERE phone = $1 AND deleted_at IS NULL
            "#,
            phone.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_user).transpose()
    }

    async fn find_by_username(&self, username: &Username) -> DomainResult<Option<User>> {
        let record = sqlx::query_as!(
            UserRecord,
            r#"
            SELECT 
                id AS "id!", 
                username, 
                phone AS "phone!", 
                phone_hash AS "phone_hash!", 
                email, 
                status_text, 
                two_fa_enabled AS "two_fa_enabled!", 
                two_fa_secret, 
                is_active AS "is_active!", 
                last_seen_at, 
                created_at AS "created_at!", 
                updated_at AS "updated_at!", 
                deleted_at 
            FROM users 
            WHERE username = $1 AND deleted_at IS NULL
            "#,
            username.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_user).transpose()
    }

    async fn update(&self, user: &User) -> DomainResult<()> {
        sqlx::query!(
            r#"
            UPDATE users SET 
                username = $2,
                phone = $3,
                phone_hash = $4,
                email = $5,
                status_text = $6,
                two_fa_enabled = $7,
                two_fa_secret = $8,
                is_active = $9,
                last_seen_at = $10,
                updated_at = NOW()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
            user.id.0,
            user.username.as_ref().map(|u| u.as_str()),
            user.phone.as_str(),
            user.phone_hash,
            user.email.as_ref().map(|e| e.as_str()),
            user.status_text,
            user.two_fa_enabled,
            user.two_fa_secret,
            user.is_active,
            user.last_seen_at
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        // Note: Update phone_hash in Redis if it changed (though phone updates are rare)
        if let Some(mut redis) = self.redis.clone() {
            use redis::AsyncCommands;
            let _: () = redis
                .sadd("phone_hashes", &user.phone_hash)
                .await
                .map_err(|e| DomainError::Internal(e.to_string()))?;
        }

        Ok(())
    }

    async fn delete_soft(&self, id: &UserId) -> DomainResult<()> {
        let user = self.find_by_id(id).await?;

        sqlx::query!(
            r#"
            UPDATE users SET deleted_at = NOW() WHERE id = $1
            "#,
            id.0
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if let (Some(u), Some(mut redis)) = (user, self.redis.clone()) {
            use redis::AsyncCommands;
            let _: () = redis
                .srem("phone_hashes", &u.phone_hash)
                .await
                .map_err(|e| DomainError::Internal(e.to_string()))?;
        }

        Ok(())
    }

    async fn update_last_seen(
        &self,
        user_id: &UserId,
        timestamp: DateTime<Utc>,
    ) -> DomainResult<()> {
        sqlx::query(
            r#"
            UPDATE users SET last_seen_at = $2 WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(user_id.0)
        .bind(timestamp)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }
}

fn map_record_to_user(rec: UserRecord) -> DomainResult<User> {
    Ok(User {
        id: UserId(rec.id),
        username: rec.username.map(Username::new).transpose()?,
        phone: PhoneNumber::new(rec.phone)?,
        phone_hash: rec.phone_hash,
        email: rec.email.map(Email::new).transpose()?,
        status_text: rec.status_text.unwrap_or_default(),
        two_fa_enabled: rec.two_fa_enabled,
        two_fa_secret: rec.two_fa_secret,
        is_active: rec.is_active,
        last_seen_at: rec.last_seen_at,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
        deleted_at: rec.deleted_at,
    })
}

impl PostgresUserRepository {
    pub async fn search_users(
        &self,
        query: &str,
        limit: i32,
        exclude_user_id: Uuid,
    ) -> DomainResult<Vec<(Uuid, Option<String>, String, Option<String>)>> {
        let search_pattern = if query.starts_with('@') {
            format!("{}%", query.trim_start_matches('@'))
        } else if query.starts_with('+') {
            query.to_string()
        } else {
            format!("%{}%", query)
        };

        let records = sqlx::query_as::<_, (Uuid, Option<String>, String, Option<String>)>(
            r#"
            SELECT u.id, u.username, COALESCE(up.display_name, ''), u.avatar_url
            FROM users u
            LEFT JOIN user_profiles up ON up.user_id = u.id
            WHERE u.deleted_at IS NULL
              AND u.is_active = TRUE
              AND u.id != $1
              AND (
                  u.username ILIKE $2
                  OR up.display_name ILIKE $2
                  OR u.phone ILIKE $3
              )
              AND NOT EXISTS (SELECT 1 FROM user_blocks ub WHERE ub.blocker_id = u.id AND ub.blocked_id = $1)
              AND NOT EXISTS (SELECT 1 FROM user_blocks ub WHERE ub.blocker_id = $1 AND ub.blocked_id = u.id)
            ORDER BY u.username NULLS LAST, up.display_name NULLS LAST
            LIMIT $4
            "#,
        )
        .bind(exclude_user_id)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(records)
    }

    pub async fn find_profile_by_id(
        &self,
        user_id: &Uuid,
    ) -> DomainResult<
        Option<(
            Uuid,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
    > {
        let record = sqlx::query_as::<_, (Uuid, Option<String>, String, Option<String>, Option<String>, Option<String>)>(
            r#"
            SELECT u.id, u.username, COALESCE(up.display_name, ''), up.bio, u.avatar_url, u.status_text
            FROM users u
            LEFT JOIN user_profiles up ON up.user_id = u.id
            WHERE u.id = $1 AND u.deleted_at IS NULL AND u.is_active = TRUE
            "#,
        )
        .bind(*user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(record)
    }

    pub async fn is_user_blocked(
        &self,
        blocker_id: &Uuid,
        blocked_id: &Uuid,
    ) -> DomainResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar(
            r#"
            SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2
            "#,
        )
        .bind(blocker_id)
        .bind(blocked_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(exists.is_some())
    }

    pub async fn block_user(&self, blocker_id: &Uuid, blocked_id: &Uuid) -> DomainResult<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO user_blocks (id, blocker_id, blocked_id)
            VALUES ($1, $2, $3)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(id)
        .bind(blocker_id)
        .bind(blocked_id)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(id)
    }

    pub async fn unblock_user(&self, blocker_id: &Uuid, blocked_id: &Uuid) -> DomainResult<bool> {
        let result =
            sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
                .bind(blocker_id)
                .bind(blocked_id)
                .execute(&self.pool)
                .await
                .map_err(DomainError::from_sqlx)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_blocked_users(&self, user_id: &Uuid) -> DomainResult<Vec<(Uuid, Uuid)>> {
        let records = sqlx::query_as::<_, (Uuid, Uuid)>(
            r#"
            SELECT id, blocked_id FROM user_blocks WHERE blocker_id = $1 ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(records)
    }

    pub async fn find_id_by_phone(&self, phone: &str) -> DomainResult<Option<Uuid>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            r#"
            SELECT id FROM users WHERE phone = $1 AND deleted_at IS NULL AND is_active = TRUE
            "#,
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(id)
    }
}
