use chrono::{DateTime, Utc};
use domain::contact::entity::Contact;
use domain::contact::repository::ContactRepository;
use domain::contact::value_objects::{ContactId, ContactPhoneNumber};
use domain::user::value_objects::{PhoneNumber, UserId};
use shared::error::{DomainError, DomainResult};
use sqlx::PgPool;

use redis::aio::ConnectionManager;

pub struct PostgresContactRepository {
    pool: PgPool,
    redis: Option<ConnectionManager>,
}

impl PostgresContactRepository {
    pub fn new(pool: PgPool, redis: Option<ConnectionManager>) -> Self {
        Self { pool, redis }
    }
}

#[derive(sqlx::FromRow)]
struct ContactRecord {
    id: Uuid,
    owner_id: Uuid,
    contact_id: Option<Uuid>,
    phone: String,
    nickname: Option<String>,
    is_favorite: bool,
    created_at: DateTime<Utc>,
}

impl ContactRepository for PostgresContactRepository {
    async fn create(&self, contact: &Contact) -> DomainResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO contacts (id, owner_id, contact_id, phone, nickname, is_favorite, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            contact.id.0,
            contact.owner_id.0,
            contact.contact_id.as_ref().map(|c| c.0),
            contact.phone.as_str(),
            contact.nickname,
            contact.is_favorite,
            contact.created_at
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    async fn find_by_id(&self, id: &ContactId) -> DomainResult<Option<Contact>> {
        let record = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE id = $1
            "#,
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_contact).transpose()
    }

    async fn find_by_owner_and_phone(
        &self,
        owner_id: &UserId,
        phone: &ContactPhoneNumber,
    ) -> DomainResult<Option<Contact>> {
        let record = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE owner_id = $1 AND phone = $2
            "#,
            owner_id.0,
            phone.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_contact).transpose()
    }

    async fn find_all_by_owner(&self, owner_id: &UserId) -> DomainResult<Vec<Contact>> {
        let records = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE owner_id = $1
            ORDER BY nickname ASC, phone ASC
            "#,
            owner_id.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        records.into_iter().map(map_record_to_contact).collect()
    }

    async fn find_favorites(&self, owner_id: &UserId) -> DomainResult<Vec<Contact>> {
        let records = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE owner_id = $1 AND is_favorite = TRUE
            ORDER BY nickname ASC, phone ASC
            "#,
            owner_id.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        records.into_iter().map(map_record_to_contact).collect()
    }

    async fn update(&self, contact: &Contact) -> DomainResult<()> {
        sqlx::query!(
            r#"
            UPDATE contacts SET 
                contact_id = $2,
                phone = $3,
                nickname = $4,
                is_favorite = $5
            WHERE id = $1
            "#,
            contact.id.0,
            contact.contact_id.as_ref().map(|c| c.0),
            contact.phone.as_str(),
            contact.nickname,
            contact.is_favorite
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    async fn delete(&self, id: &ContactId) -> DomainResult<()> {
        sqlx::query!(
            r#"
            DELETE FROM contacts WHERE id = $1
            "#,
            id.0
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    async fn delete_by_owner(&self, owner_id: &UserId) -> DomainResult<()> {
        sqlx::query!(
            r#"
            DELETE FROM contacts WHERE owner_id = $1
            "#,
            owner_id.0
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }
}

fn map_record_to_contact(rec: ContactRecord) -> DomainResult<Contact> {
    Ok(Contact {
        id: ContactId(rec.id),
        owner_id: UserId(rec.owner_id),
        contact_id: rec.contact_id.map(UserId),
        phone: PhoneNumber::new(rec.phone)?,
        nickname: rec.nickname,
        is_favorite: rec.is_favorite,
        created_at: rec.created_at,
    })
}

use domain::user::entity::User;

impl PostgresContactRepository {
    pub async fn sync_contacts(
        &self,
        _owner_id: &Uuid,
        hashes: &[String],
    ) -> DomainResult<Vec<(String, Option<String>, Option<String>, Option<String>)>> {
        if hashes.is_empty() {
            return Ok(vec![]);
        }

        let mut redis_manager = self.redis.clone().ok_or_else(|| {
            DomainError::Internal("Redis connection not available for contact sync".to_string())
        })?;

        // 1. Check which hashes exist in the "phone_hashes" set
        use redis::AsyncCommands;
        let matches: Vec<bool> = redis_manager
            .smismember("phone_hashes", hashes)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        let existing_hashes: Vec<String> = hashes
            .iter()
            .enumerate()
            .filter(|(i, _)| matches[*i])
            .map(|(_, h)| h.clone())
            .collect();

        if existing_hashes.is_empty() {
            return Ok(vec![]);
        }

        // 2. Query Postgres for the users matching those hashes
        let records =
            sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
                r#"
            SELECT 
                u.phone_hash,
                u.id::text,
                u.username,
                up.display_name
            FROM users u
            LEFT JOIN user_profiles up ON up.user_id = u.id
            WHERE u.phone_hash = ANY($1) 
              AND u.deleted_at IS NULL 
              AND u.is_active = TRUE
            "#,
            )
            .bind(&existing_hashes)
            .fetch_all(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;

        Ok(records)
    }

    pub async fn find_user_by_phone(&self, phone: &PhoneNumber) -> DomainResult<Option<User>> {
        use domain::user::entity::User;
        use domain::user::value_objects::{Email, UserId, Username};

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
            WHERE phone = $1 AND deleted_at IS NULL AND is_active = TRUE
            "#,
            phone.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record
            .map(|rec| {
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
            })
            .transpose()
    }

    pub async fn find_all_by_owner_owned(&self, owner_id: &Uuid) -> DomainResult<Vec<Contact>> {
        let records = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE owner_id = $1
            ORDER BY nickname ASC, phone ASC
            "#,
            owner_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        records.into_iter().map(map_record_to_contact).collect()
    }

    pub async fn find_by_owner_and_phone_owned(
        &self,
        owner_id: &Uuid,
        phone: &str,
    ) -> DomainResult<Option<Contact>> {
        let record = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
            FROM contacts
            WHERE owner_id = $1 AND phone = $2
            "#,
            owner_id,
            phone
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_contact).transpose()
    }

    pub async fn find_by_id_owned(&self, id: &Uuid) -> DomainResult<Option<Contact>> {
        let record = sqlx::query_as!(
            ContactRecord,
            r#"
            SELECT 
                id AS "id!", 
                owner_id AS "owner_id!", 
                contact_id, 
                phone AS "phone!", 
                nickname, 
                is_favorite AS "is_favorite!", 
                created_at AS "created_at!"
           FROM contacts
            WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        record.map(map_record_to_contact).transpose()
    }

    pub async fn create_contact(&self, contact: &Contact) -> DomainResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO contacts (id, owner_id, contact_id, phone, nickname, is_favorite, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            contact.id.0,
            contact.owner_id.0,
            contact.contact_id.as_ref().map(|c| c.0),
            contact.phone.as_str(),
            contact.nickname,
            contact.is_favorite,
            contact.created_at
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    pub async fn update_contact(&self, contact: &Contact) -> DomainResult<()> {
        sqlx::query!(
            r#"
            UPDATE contacts SET 
                contact_id = $2,
                phone = $3,
                nickname = $4,
                is_favorite = $5
            WHERE id = $1
            "#,
            contact.id.0,
            contact.contact_id.as_ref().map(|c| c.0),
            contact.phone.as_str(),
            contact.nickname,
            contact.is_favorite
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    pub async fn delete_contact(&self, id: &ContactId) -> DomainResult<()> {
        sqlx::query!(
            r#"
            DELETE FROM contacts WHERE id = $1
            "#,
            id.0
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    pub async fn create_contact_raw(
        &self,
        owner_id: Uuid,
        phone: &str,
        nickname: Option<&str>,
        contact_id: Option<Uuid>,
    ) -> DomainResult<()> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        sqlx::query!(
            r#"
            INSERT INTO contacts (id, owner_id, contact_id, phone, nickname, is_favorite, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            id,
            owner_id,
            contact_id,
            phone,
            nickname,
            false,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(())
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

use uuid::Uuid;
