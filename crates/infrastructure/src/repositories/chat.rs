use chrono::{DateTime, Utc};
use domain::chat::entity::PendingAttachment;
use domain::chat::entity::{
    Chat, ChatMessage, ChatPreview, ChatType, ParticipantDetail, ParticipantRole,
};
use domain::chat::notifications::{
    ChatSettings, NewNotification, Notification, NotificationCursor, NotificationType,
    UpdateChatSettings,
};
use domain::chat::repository::{
    ChatCursor, ChatRepository, ConfirmAttachmentInput, MessageCursor, MessageDirection,
    MessageReaction, NewMessage, NewPendingAttachment,
};
use rand::Rng;
use rand::distributions::Alphanumeric;
use shared::error::{DomainError, DomainResult};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresChatRepository {
    pool: PgPool,
}

impl PostgresChatRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[allow(clippy::type_complexity)]
fn map_message_row(
    row: (
        Uuid,
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        String,
        Option<serde_json::Value>,
        bool,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    ),
) -> ChatMessage {
    ChatMessage {
        id: row.0,
        chat_id: row.1,
        sender_id: row.2,
        reply_to_id: row.3,
        content_encrypted: row.4,
        content_iv: row.5,
        message_type: row.6,
        metadata: row.7,
        is_forwarded: row.8,
        created_at: row.9,
        edited_at: row.10,
        deleted_at: row.11,
    }
}

impl ChatRepository for PostgresChatRepository {
    async fn create_private_chat(
        &self,
        creator_id: Uuid,
        participant_id: Uuid,
    ) -> DomainResult<Chat> {
        let existing = sqlx::query_as::<_, (Uuid,)>(
            r#"
            SELECT c.id
            FROM chats c
            JOIN chat_participants cp1 ON cp1.chat_id = c.id
            JOIN chat_participants cp2 ON cp2.chat_id = c.id
            WHERE c.type = 'private'
              AND c.deleted_at IS NULL
              AND cp1.user_id = $1
              AND cp2.user_id = $2
              AND cp1.left_at IS NULL
              AND cp2.left_at IS NULL
            LIMIT 1
            "#,
        )
        .bind(creator_id)
        .bind(participant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let chat_id = if let Some((chat_id,)) = existing {
            chat_id
        } else {
            let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

            let (new_chat_id,) = sqlx::query_as::<_, (Uuid,)>(
                r#"
                INSERT INTO chats (type, created_by)
                VALUES ('private', $1)
                RETURNING id
                "#,
            )
            .bind(creator_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(DomainError::from_sqlx)?;

            sqlx::query(
                r#"
                INSERT INTO chat_participants (chat_id, user_id, role, added_by)
                VALUES
                    ($1, $2, 'member', $2),
                    ($1, $3, 'member', $2)
                "#,
            )
            .bind(new_chat_id)
            .bind(creator_id)
            .bind(participant_id)
            .execute(&mut *tx)
            .await
            .map_err(DomainError::from_sqlx)?;

            tx.commit().await.map_err(DomainError::from_sqlx)?;

            new_chat_id
        };

        self.get_chat_for_user(creator_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("chat not found".to_string()))
    }

    async fn create_group_chat(
        &self,
        creator_id: Uuid,
        name: &str,
        participant_ids: &[Uuid],
    ) -> DomainResult<Chat> {
        let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

        let (chat_id,) = sqlx::query_as::<_, (Uuid,)>(
            r#"
            INSERT INTO chats (type, name, created_by)
            VALUES ('group', $1, $2)
            RETURNING id
            "#,
        )
        .bind(name)
        .bind(creator_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        sqlx::query(
            r#"
            INSERT INTO chat_participants (chat_id, user_id, role, added_by)
            VALUES ($1, $2, 'owner', $2)
            "#,
        )
        .bind(chat_id)
        .bind(creator_id)
        .execute(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        if !participant_ids.is_empty() {
            sqlx::query(
                r#"
                INSERT INTO chat_participants (chat_id, user_id, role, added_by)
                SELECT $1, p.user_id, 'member', $2
                FROM UNNEST($3::uuid[]) AS p(user_id)
                WHERE p.user_id <> $2
                ON CONFLICT (chat_id, user_id) DO NOTHING
                "#,
            )
            .bind(chat_id)
            .bind(creator_id)
            .bind(participant_ids)
            .execute(&mut *tx)
            .await
            .map_err(DomainError::from_sqlx)?;
        }

        tx.commit().await.map_err(DomainError::from_sqlx)?;

        self.get_chat_for_user(creator_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("chat not found".to_string()))
    }

    async fn get_chat_for_user(&self, user_id: Uuid, chat_id: Uuid) -> DomainResult<Option<Chat>> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<Uuid>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT c.id, c.type::text, c.name, c.description, c.avatar_url, c.created_by, c.created_at
            FROM chats c
            JOIN chat_participants cp ON cp.chat_id = c.id
            WHERE c.id = $1
              AND cp.user_id = $2
              AND cp.left_at IS NULL
              AND c.deleted_at IS NULL
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        row.map(
            |(id, chat_type, name, description, avatar_url, created_by, created_at)| {
                let parsed_type = ChatType::from_db_str(&chat_type).ok_or_else(|| {
                    DomainError::Internal(format!("invalid chat type: {chat_type}"))
                })?;
                Ok(Chat {
                    id,
                    chat_type: parsed_type,
                    name,
                    description,
                    avatar_url,
                    created_by,
                    created_at,
                })
            },
        )
        .transpose()
    }

    async fn list_chats_for_user(
        &self,
        user_id: Uuid,
        cursor: Option<ChatCursor>,
        limit: i64,
    ) -> DomainResult<Vec<ChatPreview>> {
        let page_size = limit.clamp(1, 50);
        let has_cursor = cursor.is_some();
        let cursor_pinned = cursor.as_ref().map(|c| c.is_pinned).unwrap_or(false);
        let cursor_last_message_at = cursor.as_ref().and_then(|c| c.last_message_at);
        let cursor_chat_id = cursor.map(|c| c.chat_id);

        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                Option<String>,
                Option<String>,
                Option<Uuid>,
                Option<String>,
                Option<Uuid>,
                Option<DateTime<Utc>>,
                bool,
                i32,
                bool,
                bool,
                i64,
            ),
        >(
            r#"
            SELECT
                v.chat_id,
                v.type::text,
                v.name,
                v.avatar_url,
                v.last_message_id,
                v.last_message_encrypted,
                v.last_sender_id,
                v.last_message_at,
                COALESCE(v.is_pinned, false) AS is_pinned,
                COALESCE(v.pin_order, 0) AS pin_order,
                COALESCE(v.is_muted, false) AS is_muted,
                COALESCE(v.is_archived, false) AS is_archived,
                COALESCE(ms.unread_count, 0) AS unread_count
            FROM v_chat_previews v
            LEFT JOIN LATERAL (
                SELECT COUNT(*)::BIGINT AS unread_count
                FROM message_status ms
                JOIN messages m ON m.id = ms.message_id
                WHERE ms.user_id = $1
                  AND m.chat_id = v.chat_id
                  AND ms.status <> 'read'
                  AND m.deleted_at IS NULL
            ) ms ON TRUE
            WHERE v.user_id = $1
              AND (
                    $2::bool = false
                    OR (
                        (COALESCE(v.is_pinned, false)::int < $3::int)
                        OR (
                            COALESCE(v.is_pinned, false)::int = $3::int
                            AND COALESCE(v.last_message_at, 'epoch'::timestamptz) < COALESCE($4::timestamptz, 'epoch'::timestamptz)
                        )
                        OR (
                            COALESCE(v.is_pinned, false)::int = $3::int
                            AND COALESCE(v.last_message_at, 'epoch'::timestamptz) = COALESCE($4::timestamptz, 'epoch'::timestamptz)
                            AND v.chat_id < $5::uuid
                        )
                    )
              )
            ORDER BY
                COALESCE(v.is_pinned, false) DESC,
                COALESCE(v.last_message_at, 'epoch'::timestamptz) DESC,
                v.chat_id DESC
            LIMIT $6
            "#,
        )
        .bind(user_id)
        .bind(has_cursor)
        .bind(if cursor_pinned { 1 } else { 0 })
        .bind(cursor_last_message_at)
        .bind(cursor_chat_id.unwrap_or_else(Uuid::nil))
        .bind(page_size)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        rows.into_iter()
            .map(
                |(
                    chat_id,
                    chat_type,
                    name,
                    avatar_url,
                    last_message_id,
                    last_message_encrypted,
                    last_sender_id,
                    last_message_at,
                    is_pinned,
                    pin_order,
                    is_muted,
                    is_archived,
                    unread_count,
                )| {
                    let parsed_type = ChatType::from_db_str(&chat_type).ok_or_else(|| {
                        DomainError::Internal(format!("invalid chat type in preview: {chat_type}"))
                    })?;
                    Ok(ChatPreview {
                        chat_id,
                        chat_type: parsed_type,
                        name,
                        avatar_url,
                        last_message_id,
                        last_message_encrypted,
                        last_sender_id,
                        last_message_at,
                        is_pinned,
                        pin_order,
                        is_muted,
                        is_archived,
                        unread_count,
                    })
                },
            )
            .collect()
    }

    async fn send_message(
        &self,
        sender_id: Uuid,
        chat_id: Uuid,
        message: NewMessage,
    ) -> DomainResult<ChatMessage> {
        let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

        let chat_type = ensure_active_membership(&mut tx, sender_id, chat_id).await?;
        if chat_type == "private" {
            ensure_not_blocked_in_private_chat(&mut tx, sender_id, chat_id).await?;
        }

        if message.message_type == "text"
            && (message.content_encrypted.is_none() || message.content_iv.is_none())
        {
            return Err(DomainError::Validation(
                "text messages require content_encrypted and content_iv".to_string(),
            ));
        }

        let inserted = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                Option<Uuid>,
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                bool,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
            ),
        >(
            r#"
            INSERT INTO messages (
                chat_id, sender_id, reply_to_id, content_encrypted, content_iv,
                message_type, metadata, is_forwarded
            )
            VALUES ($1, $2, $3, $4, $5, $6::message_type, $7, $8)
            RETURNING
                id, chat_id, sender_id, reply_to_id, content_encrypted, content_iv,
                message_type::text, metadata, is_forwarded, created_at, edited_at, deleted_at
            "#,
        )
        .bind(chat_id)
        .bind(sender_id)
        .bind(message.reply_to_id)
        .bind(message.content_encrypted)
        .bind(message.content_iv)
        .bind(message.message_type)
        .bind(message.metadata)
        .bind(message.is_forwarded)
        .fetch_one(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        let message_id = inserted.0;
        sqlx::query(
            r#"
            INSERT INTO message_status (message_id, user_id, status)
            SELECT $1, cp.user_id, 'sent'
            FROM chat_participants cp
            WHERE cp.chat_id = $2
              AND cp.left_at IS NULL
            ON CONFLICT (message_id, user_id) DO NOTHING
            "#,
        )
        .bind(message_id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        tx.commit().await.map_err(DomainError::from_sqlx)?;

        Ok(map_message_row(inserted))
    }

    async fn list_messages(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
        cursor: Option<MessageCursor>,
        direction: MessageDirection,
        limit: i64,
    ) -> DomainResult<Vec<ChatMessage>> {
        ensure_active_membership_pool(&self.pool, user_id, chat_id).await?;

        let page_size = limit.clamp(1, 50);
        let cursor_ts = cursor.as_ref().map(|c| c.created_at);
        let cursor_id = cursor.as_ref().map(|c| c.message_id);

        let direction_str = match direction {
            MessageDirection::Before => "before",
            MessageDirection::After => "after",
        };

        let mut rows = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                Option<Uuid>,
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                bool,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
            ),
        >(
            r#"
            SELECT
                id, chat_id, sender_id, reply_to_id, content_encrypted, content_iv,
                message_type::text, metadata, is_forwarded, created_at, edited_at, deleted_at
            FROM messages
            WHERE chat_id = $1
              AND deleted_at IS NULL
              AND (
                    $2::timestamptz IS NULL
                    OR (
                        $4::text = 'before'
                        AND (created_at, id) < ($2::timestamptz, $3::uuid)
                    )
                    OR (
                        $4::text = 'after'
                        AND (created_at, id) > ($2::timestamptz, $3::uuid)
                    )
              )
            ORDER BY
                CASE WHEN $4::text = 'before' THEN created_at END DESC,
                CASE WHEN $4::text = 'before' THEN id END DESC,
                CASE WHEN $4::text = 'after' THEN created_at END ASC,
                CASE WHEN $4::text = 'after' THEN id END ASC
            LIMIT $5
            "#,
        )
        .bind(chat_id)
        .bind(cursor_ts)
        .bind(cursor_id.unwrap_or_else(Uuid::nil))
        .bind(direction_str)
        .bind(page_size)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if matches!(direction, MessageDirection::Before) {
            rows.reverse();
        }

        Ok(rows.into_iter().map(map_message_row).collect())
    }

    async fn create_pending_attachment(
        &self,
        input: NewPendingAttachment,
    ) -> DomainResult<PendingAttachment> {
        if let Some(chat_id) = input.chat_id {
            ensure_active_membership_pool(&self.pool, input.uploader_id, chat_id).await?;
        }

        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                String,
                String,
                String,
                i64,
                Option<String>,
                bool,
                DateTime<Utc>,
            ),
        >(
            r#"
            INSERT INTO message_attachments (
                id, message_id, uploader_id, chat_id, object_key, file_url, file_type, file_size, file_name, confirmed
            )
            VALUES ($1, NULL, $2, $3, $4, $5, $6, $7, $8, FALSE)
            RETURNING id, uploader_id, chat_id, object_key, file_url, file_type, file_size, file_name, confirmed, created_at
            "#,
        )
        .bind(input.attachment_id)
        .bind(input.uploader_id)
        .bind(input.chat_id)
        .bind(input.object_key)
        .bind(input.file_url)
        .bind(input.file_type)
        .bind(input.file_size)
        .bind(input.file_name)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(PendingAttachment {
            id: row.0,
            uploader_id: row.1,
            chat_id: row.2,
            object_key: row.3,
            file_url: row.4,
            file_type: row.5,
            file_size: row.6,
            file_name: row.7,
            confirmed: row.8,
            created_at: row.9,
        })
    }

    async fn get_pending_attachment_for_user(
        &self,
        attachment_id: Uuid,
        uploader_id: Uuid,
    ) -> DomainResult<Option<PendingAttachment>> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                String,
                String,
                String,
                i64,
                Option<String>,
                bool,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT
                id, uploader_id, chat_id, object_key, file_url, file_type, file_size, file_name, confirmed, created_at
            FROM message_attachments
            WHERE id = $1
              AND uploader_id = $2
              AND confirmed = FALSE
            "#,
        )
        .bind(attachment_id)
        .bind(uploader_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(row.map(|r| PendingAttachment {
            id: r.0,
            uploader_id: r.1,
            chat_id: r.2,
            object_key: r.3,
            file_url: r.4,
            file_type: r.5,
            file_size: r.6,
            file_name: r.7,
            confirmed: r.8,
            created_at: r.9,
        }))
    }

    async fn confirm_attachment(&self, input: ConfirmAttachmentInput) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

        let chat_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT m.chat_id
            FROM messages m
            WHERE m.id = $1
              AND m.deleted_at IS NULL
            "#,
        )
        .bind(input.message_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?
        .ok_or_else(|| DomainError::NotFound("message not found".to_string()))?;

        ensure_active_membership(&mut tx, input.uploader_id, chat_id).await?;

        let updated = sqlx::query(
            r#"
            UPDATE message_attachments
            SET message_id = $2,
                encryption_key_enc = $3,
                encryption_iv = $4,
                confirmed = TRUE,
                confirmed_at = NOW()
            WHERE id = $1
              AND uploader_id = $5
              AND chat_id = $6
              AND confirmed = FALSE
            "#,
        )
        .bind(input.attachment_id)
        .bind(input.message_id)
        .bind(input.encryption_key_enc)
        .bind(input.encryption_iv)
        .bind(input.uploader_id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        if updated.rows_affected() == 0 {
            return Err(DomainError::NotFound(
                "pending attachment not found for message".to_string(),
            ));
        }

        tx.commit().await.map_err(DomainError::from_sqlx)?;
        Ok(())
    }

    async fn verify_participant(&self, user_id: Uuid, chat_id: Uuid) -> DomainResult<()> {
        let exists = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT 1::bigint
            FROM chat_participants cp
            JOIN chats c ON c.id = cp.chat_id
            WHERE cp.chat_id = $1
              AND cp.user_id = $2
              AND cp.left_at IS NULL
              AND c.deleted_at IS NULL
            LIMIT 1
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if exists.is_none() {
            return Err(DomainError::NotFound(
                "chat not found or not a participant".to_string(),
            ));
        }
        Ok(())
    }

    async fn verify_message_in_chat(&self, message_id: Uuid, chat_id: Uuid) -> DomainResult<()> {
        let exists = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT 1::bigint
            FROM messages m
            WHERE m.id = $1
              AND m.chat_id = $2
              AND m.deleted_at IS NULL
            LIMIT 1
            "#,
        )
        .bind(message_id)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if exists.is_none() {
            return Err(DomainError::NotFound(
                "message not found or not in chat".to_string(),
            ));
        }
        Ok(())
    }

    async fn mark_messages_read(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
        up_to: DateTime<Utc>,
    ) -> DomainResult<i32> {
        let updated_count = sqlx::query_scalar::<_, i32>(
            r#"
            SELECT mark_messages_read($1, $2, $3)
            "#,
        )
        .bind(user_id)
        .bind(chat_id)
        .bind(up_to)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(updated_count)
    }

    async fn add_reaction(
        &self,
        message_id: Uuid,
        user_id: Uuid,
        reaction: String,
    ) -> DomainResult<MessageReaction> {
        let reaction_record = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, DateTime<Utc>)>(
            r#"
            INSERT INTO message_reactions (message_id, user_id, reaction)
            VALUES ($1, $2, $3)
            ON CONFLICT (message_id, user_id, reaction) DO NOTHING
            RETURNING id, message_id, user_id, reaction, created_at
            "#,
        )
        .bind(message_id)
        .bind(user_id)
        .bind(&reaction)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(MessageReaction {
            id: reaction_record.0,
            message_id: reaction_record.1,
            user_id: reaction_record.2,
            reaction: reaction_record.3,
            created_at: reaction_record.4,
        })
    }

    async fn remove_reaction(
        &self,
        message_id: Uuid,
        user_id: Uuid,
        reaction: &str,
    ) -> DomainResult<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM message_reactions
            WHERE message_id = $1
              AND user_id = $2
              AND reaction = $3
            "#,
        )
        .bind(message_id)
        .bind(user_id)
        .bind(reaction)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_chat(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
        name: Option<String>,
        description: Option<String>,
        avatar_url: Option<String>,
    ) -> DomainResult<Chat> {
        let (chat_type, current_name, current_desc, current_avatar, created_by) = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<Uuid>,
            ),
        >(
            r#"
            SELECT c.type::text, c.name, c.description, c.avatar_url, c.created_by
            FROM chats c
            JOIN chat_participants cp ON cp.chat_id = c.id
            WHERE c.id = $1
              AND cp.user_id = $2
              AND cp.left_at IS NULL
              AND c.deleted_at IS NULL
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let new_name = name.or(current_name);
        let new_desc = description.or(current_desc);
        let new_avatar = avatar_url.or(current_avatar);

        let (id, updated_at) = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
            r#"
            UPDATE chats
            SET name = $2, description = $3, avatar_url = $4, updated_at = NOW()
            WHERE id = $1
            RETURNING id, updated_at
            "#,
        )
        .bind(chat_id)
        .bind(new_name.clone())
        .bind(new_desc.clone())
        .bind(new_avatar.clone())
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let parsed_type = ChatType::from_db_str(&chat_type)
            .ok_or_else(|| DomainError::Internal("invalid chat type".to_string()))?;

        Ok(Chat {
            id,
            chat_type: parsed_type,
            name: new_name,
            description: new_desc,
            avatar_url: new_avatar,
            created_by,
            created_at: updated_at,
        })
    }

    async fn delete_chat(&self, user_id: Uuid, chat_id: Uuid) -> DomainResult<()> {
        let chat_exists = sqlx::query_scalar::<_, String>(
            r#"
            SELECT type::text
            FROM chats c
            JOIN chat_participants cp ON cp.chat_id = c.id
            WHERE c.id = $1
              AND cp.user_id = $2
              AND cp.left_at IS NULL
              AND c.deleted_at IS NULL
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let chat_type = chat_exists.ok_or_else(|| {
            DomainError::NotFound("chat not found or not a participant".to_string())
        })?;

        if chat_type == "private" {
            sqlx::query(
                r#"
                UPDATE chat_participants
                SET left_at = NOW()
                WHERE chat_id = $1 AND user_id = $2
                "#,
            )
            .bind(chat_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;
        } else {
            let is_owner = sqlx::query_scalar::<_, bool>(
                r#"
                SELECT TRUE
                FROM chat_participants
                WHERE chat_id = $1 AND user_id = $2 AND role = 'owner'
                "#,
            )
            .bind(chat_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;

            if is_owner.unwrap_or(false) {
                sqlx::query(
                    r#"
                    UPDATE chats SET deleted_at = NOW() WHERE id = $1
                    "#,
                )
                .bind(chat_id)
                .execute(&self.pool)
                .await
                .map_err(DomainError::from_sqlx)?;
            } else {
                sqlx::query(
                    r#"
                    UPDATE chat_participants SET left_at = NOW()
                    WHERE chat_id = $1 AND user_id = $2
                    "#,
                )
                .bind(chat_id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(DomainError::from_sqlx)?;
            }
        }

        Ok(())
    }

    async fn edit_message(
        &self,
        user_id: Uuid,
        message_id: Uuid,
        content_encrypted: Option<String>,
        content_iv: Option<String>,
    ) -> DomainResult<ChatMessage> {
        let chat_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT m.chat_id
            FROM messages m
            WHERE m.id = $1
              AND m.sender_id = $2
              AND m.deleted_at IS NULL
            "#,
        )
        .bind(message_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?
        .ok_or_else(|| DomainError::NotFound("message not found or not authorized".to_string()))?;

        ensure_active_membership_pool(&self.pool, user_id, chat_id).await?;

        if content_encrypted.is_none() && content_iv.is_none() {
            return Err(DomainError::Validation(
                "content_encrypted and content_iv are required".to_string(),
            ));
        }

        let updated = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                Option<Uuid>,
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                bool,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
            ),
        >(
            r#"
            UPDATE messages
            SET content_encrypted = COALESCE($3, content_encrypted),
                content_iv = COALESCE($4, content_iv),
                edited_at = NOW()
            WHERE id = $1
            RETURNING id, chat_id, sender_id, reply_to_id, content_encrypted, content_iv,
                      message_type::text, metadata, is_forwarded, created_at, edited_at, deleted_at
            "#,
        )
        .bind(message_id)
        .bind(user_id)
        .bind(content_encrypted)
        .bind(content_iv)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(map_message_row(updated))
    }

    async fn delete_message(&self, user_id: Uuid, message_id: Uuid) -> DomainResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE messages
            SET deleted_at = NOW(),
                content_encrypted = NULL,
                content_iv = NULL,
                message_type = 'deleted'::message_type
            WHERE id = $1
              AND sender_id = $2
              AND deleted_at IS NULL
            "#,
        )
        .bind(message_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::NotFound(
                "message not found or not authorized".to_string(),
            ));
        }

        Ok(())
    }

    async fn get_chat_participants(&self, chat_id: Uuid) -> DomainResult<Vec<Uuid>> {
        let participants = sqlx::query_as::<_, (Uuid,)>(
            r#"
            SELECT user_id
            FROM chat_participants
            WHERE chat_id = $1 AND left_at IS NULL
            "#,
        )
        .bind(chat_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(participants.into_iter().map(|(id,)| id).collect())
    }

    async fn list_notifications(
        &self,
        user_id: Uuid,
        cursor: Option<NotificationCursor>,
        limit: i64,
    ) -> DomainResult<Vec<Notification>> {
        let page_size = limit.clamp(1, 50);
        let cursor_ts = cursor.as_ref().map(|c| c.created_at);
        let cursor_id = cursor.as_ref().map(|c| c.id);

        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                String,
                serde_json::Value,
                bool,
                Option<DateTime<Utc>>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT id, user_id, type::text, data, is_read, read_at, created_at
            FROM notifications
            WHERE user_id = $1
              AND (
                  $2::timestamptz IS NULL
                  OR (created_at, id) < ($2::timestamptz, $3::uuid)
              )
            ORDER BY created_at DESC, id DESC
            LIMIT $4
            "#,
        )
        .bind(user_id)
        .bind(cursor_ts)
        .bind(cursor_id.unwrap_or_else(Uuid::nil))
        .bind(page_size)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        rows.into_iter()
            .map(
                |(id, user_id, notification_type, data, is_read, read_at, created_at)| {
                    let nt =
                        NotificationType::from_db_str(&notification_type).ok_or_else(|| {
                            DomainError::Internal(format!(
                                "invalid notification type: {notification_type}"
                            ))
                        })?;
                    Ok(Notification {
                        id,
                        user_id,
                        notification_type: nt,
                        data,
                        is_read,
                        read_at,
                        created_at,
                    })
                },
            )
            .collect()
    }

    async fn create_notification(
        &self,
        notification: NewNotification,
    ) -> DomainResult<Notification> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                String,
                serde_json::Value,
                bool,
                Option<DateTime<Utc>>,
                DateTime<Utc>,
            ),
        >(
            r#"
            INSERT INTO notifications (user_id, type, data)
            VALUES ($1, $2::notification_type, $3)
            RETURNING id, user_id, type::text, data, is_read, read_at, created_at
            "#,
        )
        .bind(notification.user_id)
        .bind(notification.notification_type.as_db_str())
        .bind(notification.data)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let nt = NotificationType::from_db_str(&row.2)
            .ok_or_else(|| DomainError::Internal("invalid notification type".to_string()))?;

        Ok(Notification {
            id: row.0,
            user_id: row.1,
            notification_type: nt,
            data: row.3,
            is_read: row.4,
            read_at: row.5,
            created_at: row.6,
        })
    }

    async fn mark_notification_read(
        &self,
        user_id: Uuid,
        notification_id: Uuid,
    ) -> DomainResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE notifications
            SET is_read = true, read_at = NOW()
            WHERE id = $1 AND user_id = $2 AND is_read = false
            "#,
        )
        .bind(notification_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::NotFound(
                "notification not found or already read".to_string(),
            ));
        }
        Ok(())
    }

    async fn mark_all_notifications_read(&self, user_id: Uuid) -> DomainResult<i32> {
        let count = sqlx::query_scalar::<_, i32>(
            r#"
            WITH updated AS (
                UPDATE notifications
                SET is_read = true, read_at = NOW()
                WHERE user_id = $1 AND is_read = false
                RETURNING 1
            )
            SELECT COUNT(*)::int FROM updated
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(count)
    }

    async fn delete_read_notifications(&self, user_id: Uuid) -> DomainResult<i32> {
        let count = sqlx::query_scalar::<_, i32>(
            r#"
            WITH deleted AS (
                DELETE FROM notifications
                WHERE user_id = $1 AND is_read = true
                RETURNING 1
            )
            SELECT COUNT(*)::int FROM deleted
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(count)
    }

    async fn get_chat_settings(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
    ) -> DomainResult<Option<ChatSettings>> {
        let row = sqlx::query_as::<_, (bool, Option<DateTime<Utc>>, bool, i32, bool)>(
            r#"
            SELECT is_muted, muted_until, is_pinned, pin_order, is_archived
            FROM chat_settings
            WHERE user_id = $1 AND chat_id = $2
            "#,
        )
        .bind(user_id)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(row.map(
            |(is_muted, muted_until, is_pinned, pin_order, is_archived)| ChatSettings {
                is_muted,
                muted_until,
                is_pinned,
                pin_order,
                is_archived,
            },
        ))
    }

    async fn update_chat_settings(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
        settings: UpdateChatSettings,
    ) -> DomainResult<ChatSettings> {
        sqlx::query(
            r#"
            INSERT INTO chat_settings (user_id, chat_id, is_muted, muted_until, is_pinned, pin_order, is_archived)
            VALUES ($1, $2, COALESCE($3, FALSE), $4, COALESCE($5, FALSE), COALESCE($6, 0), COALESCE($7, FALSE))
            ON CONFLICT (user_id, chat_id) DO UPDATE SET
                is_muted = COALESCE($3, chat_settings.is_muted),
                muted_until = COALESCE($4, chat_settings.muted_until),
                is_pinned = COALESCE($5, chat_settings.is_pinned),
                pin_order = COALESCE($6, chat_settings.pin_order),
                is_archived = COALESCE($7, chat_settings.is_archived),
                updated_at = NOW()
            "#,
        )
        .bind(user_id)
        .bind(chat_id)
        .bind(settings.is_muted)
        .bind(settings.muted_until)
        .bind(settings.is_pinned)
        .bind(settings.pin_order)
        .bind(settings.is_archived)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        self.get_chat_settings(user_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("chat settings not found".to_string()))
    }

    // -------------------------------------------------------------------------
    // Phase 9 — Group / Channel management
    // -------------------------------------------------------------------------

    async fn get_participant_role(
        &self,
        user_id: Uuid,
        chat_id: Uuid,
    ) -> DomainResult<Option<ParticipantRole>> {
        let row = sqlx::query_scalar::<_, String>(
            r#"
            SELECT role::text
            FROM chat_participants
            WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(row.and_then(|s| ParticipantRole::from_db_str(&s)))
    }

    async fn get_participants_detail(
        &self,
        actor_id: Uuid,
        chat_id: Uuid,
    ) -> DomainResult<Vec<ParticipantDetail>> {
        // Verify actor is a member first
        self.verify_participant(actor_id, chat_id).await?;

        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                String,
                Option<String>,
                Option<Uuid>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT user_id, chat_id, role::text, encryption_key_enc, added_by, joined_at
            FROM chat_participants
            WHERE chat_id = $1 AND left_at IS NULL
            ORDER BY joined_at ASC
            "#,
        )
        .bind(chat_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        rows.into_iter()
            .map(
                |(user_id, chat_id, role_str, encryption_key_enc, added_by, joined_at)| {
                    let role = ParticipantRole::from_db_str(&role_str).ok_or_else(|| {
                        DomainError::Internal(format!("invalid participant role: {role_str}"))
                    })?;
                    Ok(ParticipantDetail {
                        user_id,
                        chat_id,
                        role,
                        encryption_key_enc,
                        added_by,
                        joined_at,
                    })
                },
            )
            .collect()
    }

    async fn add_participant(
        &self,
        chat_id: Uuid,
        actor_id: Uuid,
        user_id: Uuid,
        encryption_key_enc: Option<String>,
    ) -> DomainResult<ParticipantDetail> {
        // Actor must be admin or owner
        let actor_role = self
            .get_participant_role(actor_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        if actor_role < ParticipantRole::Admin {
            return Err(DomainError::Unauthorized(
                "only admins and owners can add participants".to_string(),
            ));
        }

        // Check the chat is a group or channel (not private)
        let chat_type = sqlx::query_scalar::<_, String>(
            "SELECT type::text FROM chats WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?
        .ok_or_else(|| DomainError::NotFound("chat not found".to_string()))?;

        if chat_type == "private" {
            return Err(DomainError::Validation(
                "cannot add participants to a private chat".to_string(),
            ));
        }

        // Check not already a member
        let already_member = sqlx::query_scalar::<_, i64>(
            "SELECT 1::bigint FROM chat_participants WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL",
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if already_member.is_some() {
            return Err(DomainError::Validation(
                "user is already a member".to_string(),
            ));
        }

        let row = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>, Option<Uuid>, DateTime<Utc>)>(
            r#"
            INSERT INTO chat_participants (chat_id, user_id, role, encryption_key_enc, added_by, joined_at)
            VALUES ($1, $2, 'member'::participant_role, $3, $4, NOW())
            ON CONFLICT (chat_id, user_id) DO UPDATE
                SET left_at = NULL, role = 'member'::participant_role,
                    encryption_key_enc = EXCLUDED.encryption_key_enc,
                    added_by = EXCLUDED.added_by,
                    joined_at = NOW()
            RETURNING user_id, chat_id, role::text, encryption_key_enc, added_by, joined_at
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .bind(encryption_key_enc)
        .bind(actor_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let role = ParticipantRole::from_db_str(&row.2)
            .ok_or_else(|| DomainError::Internal("invalid role".to_string()))?;
        Ok(ParticipantDetail {
            user_id: row.0,
            chat_id: row.1,
            role,
            encryption_key_enc: row.3,
            added_by: row.4,
            joined_at: row.5,
        })
    }

    async fn remove_participant(
        &self,
        chat_id: Uuid,
        actor_id: Uuid,
        target_id: Uuid,
    ) -> DomainResult<bool> {
        let actor_role = self
            .get_participant_role(actor_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        let target_role = self
            .get_participant_role(target_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("target user is not a member".to_string()))?;

        // Self-leave: anyone can leave, but owners must transfer first
        if actor_id == target_id {
            if actor_role == ParticipantRole::Owner {
                return Err(DomainError::Validation(
                    "owner must transfer ownership before leaving".to_string(),
                ));
            }
        } else {
            // Kicking: actor must outrank target
            if actor_role <= target_role {
                return Err(DomainError::Unauthorized(
                    "insufficient role to remove this participant".to_string(),
                ));
            }
        }

        sqlx::query(
            "UPDATE chat_participants SET left_at = NOW() WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL",
        )
        .bind(chat_id)
        .bind(target_id)
        .execute(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        // Key rotation is always required when a member leaves
        Ok(true)
    }

    async fn update_participant_role(
        &self,
        chat_id: Uuid,
        actor_id: Uuid,
        target_id: Uuid,
        new_role: ParticipantRole,
    ) -> DomainResult<ParticipantDetail> {
        let actor_role = self
            .get_participant_role(actor_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        let target_role = self
            .get_participant_role(target_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::NotFound("target user is not a member".to_string()))?;

        // Actor must outrank both the target's current role and the new role
        if actor_role <= target_role || actor_role <= new_role {
            return Err(DomainError::Unauthorized(
                "insufficient role to change this participant's role".to_string(),
            ));
        }

        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                String,
                Option<String>,
                Option<Uuid>,
                DateTime<Utc>,
            ),
        >(
            r#"
            UPDATE chat_participants
            SET role = $3::participant_role
            WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL
            RETURNING user_id, chat_id, role::text, encryption_key_enc, added_by, joined_at
            "#,
        )
        .bind(chat_id)
        .bind(target_id)
        .bind(new_role.as_db_str())
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let role = ParticipantRole::from_db_str(&row.2)
            .ok_or_else(|| DomainError::Internal("invalid role returned".to_string()))?;
        Ok(ParticipantDetail {
            user_id: row.0,
            chat_id: row.1,
            role,
            encryption_key_enc: row.3,
            added_by: row.4,
            joined_at: row.5,
        })
    }

    async fn set_invite_link(
        &self,
        chat_id: Uuid,
        actor_id: Uuid,
        slug: Option<String>,
    ) -> DomainResult<Option<String>> {
        let actor_role = self
            .get_participant_role(actor_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        if actor_role < ParticipantRole::Admin {
            return Err(DomainError::Unauthorized(
                "only admins and owners can manage invite links".to_string(),
            ));
        }

        let new_slug = match slug.as_deref() {
            Some("") | None => {
                // Generate a random 12-character slug
                let s: String = rand::thread_rng()
                    .sample_iter(&Alphanumeric)
                    .take(12)
                    .map(char::from)
                    .collect();
                Some(s.to_lowercase())
            }
            Some(s) => Some(s.to_string()),
        };

        sqlx::query("UPDATE chats SET invite_link = $2, updated_at = NOW() WHERE id = $1")
            .bind(chat_id)
            .bind(&new_slug)
            .execute(&self.pool)
            .await
            .map_err(DomainError::from_sqlx)?;

        Ok(new_slug)
    }

    async fn find_chat_by_slug(&self, slug: &str) -> DomainResult<Option<Chat>> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<Uuid>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT id, type::text, name, description, avatar_url, created_by, created_at
            FROM chats
            WHERE invite_link = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        row.map(
            |(id, chat_type, name, description, avatar_url, created_by, created_at)| {
                let parsed_type = ChatType::from_db_str(&chat_type).ok_or_else(|| {
                    DomainError::Internal(format!("invalid chat type: {chat_type}"))
                })?;
                Ok(Chat {
                    id,
                    chat_type: parsed_type,
                    name,
                    description,
                    avatar_url,
                    created_by,
                    created_at,
                })
            },
        )
        .transpose()
    }

    async fn join_by_invite(
        &self,
        chat_id: Uuid,
        user_id: Uuid,
    ) -> DomainResult<ParticipantDetail> {
        // Check not already a member
        let already_member = sqlx::query_scalar::<_, i64>(
            "SELECT 1::bigint FROM chat_participants WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL",
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        if already_member.is_some() {
            return Err(DomainError::Validation(
                "already a member of this chat".to_string(),
            ));
        }

        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                String,
                Option<String>,
                Option<Uuid>,
                DateTime<Utc>,
            ),
        >(
            r#"
            INSERT INTO chat_participants (chat_id, user_id, role, joined_at)
            VALUES ($1, $2, 'member'::participant_role, NOW())
            ON CONFLICT (chat_id, user_id) DO UPDATE
                SET left_at = NULL, role = 'member'::participant_role, joined_at = NOW()
            RETURNING user_id, chat_id, role::text, encryption_key_enc, added_by, joined_at
            "#,
        )
        .bind(chat_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        let role = ParticipantRole::from_db_str(&row.2)
            .ok_or_else(|| DomainError::Internal("invalid role".to_string()))?;
        Ok(ParticipantDetail {
            user_id: row.0,
            chat_id: row.1,
            role,
            encryption_key_enc: row.3,
            added_by: row.4,
            joined_at: row.5,
        })
    }

    async fn rotate_group_key(
        &self,
        chat_id: Uuid,
        actor_id: Uuid,
        keys: Vec<(Uuid, String)>,
    ) -> DomainResult<usize> {
        let actor_role = self
            .get_participant_role(actor_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        if actor_role < ParticipantRole::Admin {
            return Err(DomainError::Unauthorized(
                "only admins and owners can rotate the group key".to_string(),
            ));
        }

        let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

        let mut updated = 0usize;
        for (member_id, new_key) in &keys {
            let result = sqlx::query(
                r#"
                UPDATE chat_participants
                SET encryption_key_enc = $3
                WHERE chat_id = $1 AND user_id = $2 AND left_at IS NULL
                "#,
            )
            .bind(chat_id)
            .bind(member_id)
            .bind(new_key)
            .execute(&mut *tx)
            .await
            .map_err(DomainError::from_sqlx)?;

            updated += result.rows_affected() as usize;
        }

        tx.commit().await.map_err(DomainError::from_sqlx)?;

        Ok(updated)
    }

    async fn transfer_ownership(
        &self,
        chat_id: Uuid,
        current_owner_id: Uuid,
        new_owner_id: Uuid,
    ) -> DomainResult<()> {
        // Verify current owner
        let owner_role = self
            .get_participant_role(current_owner_id, chat_id)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("not a member of this chat".to_string()))?;

        if owner_role != ParticipantRole::Owner {
            return Err(DomainError::Unauthorized(
                "only the owner can transfer ownership".to_string(),
            ));
        }

        // Verify new owner is an active member
        let new_role = self
            .get_participant_role(new_owner_id, chat_id)
            .await?
            .ok_or_else(|| {
                DomainError::NotFound("new owner is not a member of this chat".to_string())
            })?;

        let _ = new_role; // any role is fine — we'll promote them

        let mut tx = self.pool.begin().await.map_err(DomainError::from_sqlx)?;

        // Demote current owner to admin
        sqlx::query(
            "UPDATE chat_participants SET role = 'admin'::participant_role WHERE chat_id = $1 AND user_id = $2",
        )
        .bind(chat_id)
        .bind(current_owner_id)
        .execute(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        // Promote new owner
        sqlx::query(
            "UPDATE chat_participants SET role = 'owner'::participant_role WHERE chat_id = $1 AND user_id = $2",
        )
        .bind(chat_id)
        .bind(new_owner_id)
        .execute(&mut *tx)
        .await
        .map_err(DomainError::from_sqlx)?;

        tx.commit().await.map_err(DomainError::from_sqlx)?;

        Ok(())
    }

    async fn list_messages_since(
        &self,
        user_id: Uuid,
        since: DateTime<Utc>,
        limit: i64,
    ) -> DomainResult<Vec<ChatMessage>> {
        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                Uuid,
                Option<Uuid>,
                Option<Uuid>,
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                bool,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
            ),
        >(
            r#"
            SELECT m.id, m.chat_id, m.sender_id, m.reply_to_id,
                   m.content_encrypted, m.content_iv, m.message_type::text,
                   m.metadata, m.is_forwarded, m.created_at, m.edited_at, m.deleted_at
            FROM messages m
            JOIN chat_participants cp ON cp.chat_id = m.chat_id
            WHERE cp.user_id = $1
              AND cp.left_at IS NULL
              AND m.created_at > $2
              AND m.deleted_at IS NULL
            ORDER BY m.created_at ASC
            LIMIT $3
            "#,
        )
        .bind(user_id)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DomainError::from_sqlx)?;

        Ok(rows.into_iter().map(map_message_row).collect())
    }
}

async fn ensure_active_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    chat_id: Uuid,
) -> DomainResult<String> {
    let chat_type = sqlx::query_scalar::<_, String>(
        r#"
        SELECT c.type::text
        FROM chats c
        JOIN chat_participants cp ON cp.chat_id = c.id
        WHERE c.id = $1
          AND c.deleted_at IS NULL
          AND cp.user_id = $2
          AND cp.left_at IS NULL
        "#,
    )
    .bind(chat_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DomainError::from_sqlx)?;

    chat_type
        .ok_or_else(|| DomainError::NotFound("chat not found or not a participant".to_string()))
}

async fn ensure_active_membership_pool(
    pool: &PgPool,
    user_id: Uuid,
    chat_id: Uuid,
) -> DomainResult<()> {
    let exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1::bigint
        FROM chat_participants cp
        JOIN chats c ON c.id = cp.chat_id
        WHERE cp.chat_id = $1
          AND cp.user_id = $2
          AND cp.left_at IS NULL
          AND c.deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(chat_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(DomainError::from_sqlx)?;

    if exists.is_none() {
        return Err(DomainError::NotFound(
            "chat not found or not a participant".to_string(),
        ));
    }
    Ok(())
}

async fn ensure_not_blocked_in_private_chat(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sender_id: Uuid,
    chat_id: Uuid,
) -> DomainResult<()> {
    let blocked = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1::bigint
        FROM chat_participants cp
        JOIN user_blocks ub
          ON (ub.blocker_id = $1 AND ub.blocked_id = cp.user_id)
          OR (ub.blocked_id = $1 AND ub.blocker_id = cp.user_id)
        WHERE cp.chat_id = $2
          AND cp.user_id <> $1
          AND cp.left_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(sender_id)
    .bind(chat_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DomainError::from_sqlx)?;

    if blocked.is_some() {
        return Err(DomainError::Unauthorized(
            "cannot send messages in blocked private chat".to_string(),
        ));
    }
    Ok(())
}
