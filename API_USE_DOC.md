# Mesty Messenger API — Usage Reference

> **Version:** 1.0  
> **Base URL:** `https://<your-domain>` (local: `http://localhost:3000`)  
> **Content-Type:** `application/json` for all requests/responses unless specified.  
> **Auth scheme:** Bearer JWT — add `Authorization: Bearer <access_token>` to every protected route.

---

## Table of Contents

1. [General Conventions](#1-general-conventions)
2. [Authentication & Session Management](#2-authentication--session-management)
3. [Cryptographic Keys (E2E Encryption)](#3-cryptographic-keys-e2e-encryption)
4. [Chats](#4-chats)
5. [Messages](#5-messages)
6. [Message Reactions](#6-message-reactions)
7. [Notifications](#7-notifications)
8. [Attachments](#8-attachments)
9. [Group Management](#9-group-management)
10. [Stories](#10-stories)
11. [Users & Profiles](#11-users--profiles)
12. [Contacts](#12-contacts)
13. [User Blocks](#13-user-blocks)
14. [WebSocket (Real-time)](#14-websocket-real-time)
15. [Utility Endpoints](#15-utility-endpoints)
16. [WebRTC & Calling](#16-webrtc--calling)
17. [Error Reference](#17-error-reference)

---

## 1. General Conventions

### Authentication

All routes marked 🔒 require a valid `Authorization: Bearer <access_token>` header.  
Tokens are short-lived; use the refresh flow when you receive a `401`.

### Pagination (cursor-based)

List endpoints that support pagination return:

```json
{
  "items": [...],
  "next_cursor": "<opaque string | null>",
  "has_more": true
}
```

Pass `?cursor=<next_cursor>&limit=<n>` on subsequent requests. Default and max limits vary per endpoint.

### Date/time format

All timestamps are **ISO 8601 / RFC 3339** strings in UTC, e.g. `"2025-04-17T14:09:58Z"`.

### Phone numbers

Must be in **E.164** format: `+<country_code><number>` e.g. `+15551234567`.

### UUIDs

All entity IDs (`id`, `user_id`, `chat_id`, etc.) are **UUIDv4** strings.

---

## 2. Authentication & Session Management

### 2.1 Register — Request OTP

Starts the registration flow for a new phone number. Sends an OTP (One-Time Password) via SMS.  
If the phone already has an account, the response is identical (no info leak).

```
POST /auth/register
```

**Rate limit:** 5 attempts / device / hour.

**Request body:**

```json
{
  "phone": "+15551234567",
  "device_id": "unique-device-uuid",
  "device_name": "Alice's iPhone",
  "device_type": "ios"
}
```

| Field         | Type   | Required | Notes                         |
| ------------- | ------ | -------- | ----------------------------- |
| `phone`       | string | ✅       | E.164 format                  |
| `device_id`   | string | ✅       | Stable per-device identifier  |
| `device_name` | string | ✅       | Human-readable label          |
| `device_type` | string | ✅       | `ios`, `android`, `web`, etc. |

**Response `202 Accepted`:**

```json
{ "message": "Código enviado" }
```

---

### 2.2 Verify Phone — Complete Registration

Verifies the OTP sent during registration, creates the user account, and issues tokens.

```
POST /auth/verify-phone
```

**Request body:**

```json
{
  "phone": "+15551234567",
  "code": "123456",
  "device_id": "unique-device-uuid",
  "device_name": "Alice's iPhone",
  "device_type": "ios",
  "push_token": "fcm-or-apns-token"
}
```

| Field         | Type   | Required | Notes                                  |
| ------------- | ------ | -------- | -------------------------------------- |
| `phone`       | string | ✅       | Must match what was sent in `register` |
| `code`        | string | ✅       | 6-digit OTP received via SMS           |
| `device_id`   | string | ❌       | Recommended; falls back to random UUID |
| `device_name` | string | ❌       | Falls back to `"unknown device"`       |
| `device_type` | string | ❌       | Falls back to `"web"`                  |
| `push_token`  | string | ❌       | FCM/APNs token for push notifications  |

**Response `201 Created`:**

```json
{
  "access_token": "<jwt>",
  "refresh_token": "<jwt>",
  "expires_in": 900,
  "user": {
    "id": "uuid",
    "phone": "+15551234567",
    "username": null
  }
}
```

> `expires_in` is the access token TTL in seconds.

---

### 2.3 Login — Request OTP

Sends a login OTP to an existing registered phone number.

```
POST /auth/login
```

**Rate limit:** 10 attempts / device / hour.

**Request body:**

```json
{
  "phone": "+15551234567",
  "device_id": "unique-device-uuid",
  "device_name": "Alice's iPhone",
  "device_type": "ios",
  "push_token": "fcm-or-apns-token"
}
```

**Response `202 Accepted`:**

```json
{ "message": "Código enviado" }
```

---

### 2.4 Login Verify — Complete Login

Verifies the login OTP and issues tokens. If 2FA is enabled, returns a temporary token instead.

```
POST /auth/login/verify
```

**Request body:**

```json
{
  "phone": "+15551234567",
  "code": "123456",
  "device_id": "unique-device-uuid",
  "device_name": "Alice's iPhone",
  "device_type": "ios",
  "push_token": "fcm-or-apns-token"
}
```

**Response `200 OK` (2FA disabled):** Same as `verify-phone` above.

**Response `202 Accepted` (2FA enabled):**

```json
{
  "two_fa_required": true,
  "temp_token": "<short-lived jwt>"
}
```

Proceed to [2FA Verify](#26-2fa-verify) with the `temp_token`.

---

### 2.5 Token Refresh

Exchanges a valid refresh token for a new access + refresh token pair. Invalidates the old refresh token.

```
POST /auth/refresh
```

**Request body:**

```json
{ "refresh_token": "<jwt>" }
```

**Response `200 OK`:** Same shape as `verify-phone`.

---

### 2.6 Account Recovery — Request OTP

Sends a recovery OTP if the account exists. Always returns the same response to prevent enumeration.

```
POST /auth/recover
```

**Request body:**

```json
{ "phone": "+15551234567" }
```

**Response `202 Accepted`:**

```json
{ "message": "Si la cuenta existe, enviamos un codigo de recuperacion" }
```

---

### 2.7 Account Recovery — Verify OTP

Verifies the recovery OTP and returns a short-lived `recover_token` to re-authenticate.

```
POST /auth/recover/verify
```

**Request body:**

```json
{
  "phone": "+15551234567",
  "code": "123456"
}
```

**Response `200 OK`:**

```json
{ "recover_token": "<jwt>" }
```

> Use the `recover_token` as you would a `temp_token` to issue full session tokens for the user (implementation-dependent on front-end flow).

---

### 2.8 2FA Setup — Generate Code 🔒

Initiates 2FA setup for the authenticated user. Returns the setup OTP (in development environments).

```
POST /auth/2fa/setup
```

**Response `200 OK`:**

```json
{
  "message": "2FA setup code generated",
  "code": "123456"
}
```

> In production the `code` should be delivered via a secondary channel (SMS/email). The response includes it directly during development.

---

### 2.9 2FA Setup Verify — Enable 2FA 🔒

Confirms the 2FA setup OTP and activates two-factor authentication on the account.

```
POST /auth/2fa/setup/verify
```

**Request body:**

```json
{ "code": "123456" }
```

**Response `200 OK`:**

```json
{ "message": "2FA habilitado" }
```

---

### 2.10 2FA Verify — Complete Login with 2FA

Verifies the 2FA challenge OTP during login using the `temp_token` from step [2.4](#24-login-verify--complete-login).

```
POST /auth/2fa/verify
```

**Request body:**

```json
{
  "temp_token": "<jwt from login/verify when 2fa_required>",
  "code": "123456",
  "device_id": "unique-device-uuid",
  "device_name": "Alice's iPhone",
  "device_type": "ios",
  "push_token": "fcm-or-apns-token"
}
```

**Response `200 OK`:** Same shape as `verify-phone`.

---

### 2.11 Logout 🔒

Revokes the current session and invalidates its tokens.

```
POST /auth/logout
```

No request body needed.

**Response `200 OK`:**

```json
{ "message": "Logout exitoso" }
```

---

### 2.12 List Sessions 🔒

Returns all active sessions for the authenticated user.

```
GET /auth/sessions
```

**Response `200 OK`:**

```json
{
  "sessions": [
    {
      "id": "session-uuid",
      "device_name": "Alice's iPhone",
      "device_type": "ios",
      "ip_address": "203.0.113.42",
      "last_active_at": "2025-04-17T12:00:00Z",
      "is_current": true
    }
  ]
}
```

---

### 2.13 Delete Session 🔒

Revokes a specific session (remote logout from another device).

```
DELETE /auth/sessions/:session_id
```

**Path param:** `session_id` — UUID of the session to revoke.

**Response `200 OK`:**

```json
{ "message": "Sesion eliminada" }
```

---

## 3. Cryptographic Keys (E2E Encryption)

The API uses the **Signal Protocol** key bundle structure for end-to-end encryption.

### 3.1 Upload Key Bundle 🔒

Uploads or replaces the full key bundle for the authenticated user (identity key, signed pre-key, and one-time pre-keys).

```
POST /keys/upload
```

**Request body:**

```json
{
  "identity_key": "<base64-encoded-public-key>",
  "signed_prekey": {
    "id": 1,
    "key": "<base64-encoded-public-key>",
    "signature": "<base64-encoded-signature>"
  },
  "one_time_prekeys": [
    { "id": 1, "key": "<base64-encoded-public-key>" },
    { "id": 2, "key": "<base64-encoded-public-key>" }
  ]
}
```

**Response `200 OK`:**

```json
{ "prekey_count": 50 }
```

---

### 3.2 Upload One-Time Pre-keys 🔒

Replenishes the one-time pre-key pool without replacing the identity or signed pre-key.

```
POST /keys/upload-prekeys
```

**Request body:**

```json
{
  "one_time_prekeys": [{ "id": 51, "key": "<base64-encoded-public-key>" }]
}
```

**Response `200 OK`:**

```json
{ "prekey_count": 51 }
```

---

### 3.3 Get Pre-key Pool Count 🔒

Returns the number of one-time pre-keys remaining in the server pool for the authenticated user.

```
GET /keys/me/count
```

**Response `200 OK`:**

```json
{ "count": 42 }
```

> Replenish when count drops below a safe threshold (e.g., 10).

---

### 3.4 Fetch Key Bundle for User 🔒

Retrieves the key bundle needed to initiate an encrypted session with another user. Consumes one one-time pre-key (if available).

```
GET /keys/:user_id
```

**Path param:** `user_id` — UUID of the target user.

**Response `200 OK`:**

```json
{
  "identity_key": "<base64>",
  "signed_prekey": {
    "id": 1,
    "key": "<base64>",
    "signature": "<base64>"
  },
  "one_time_prekey": {
    "id": 7,
    "key": "<base64>"
  }
}
```

> `one_time_prekey` may be `null` if the pool is exhausted for that user.

---

### 3.5 Get Safety Number / Fingerprint 🔒

Returns the computed safety number (fingerprint) for a session between the authenticated user and another user. Used for out-of-band verification.

```
GET /keys/:user_id/fingerprint
```

**Path param:** `user_id` — UUID of the other party.

**Response `200 OK`:**

```json
{
  "fingerprint": "0123 4567 8901 2345 6789 0123 4567 8901 2345 6789 0123 4567",
  "your_key": "<base64-identity-key>",
  "their_key": "<base64-identity-key>",
  "key_changed": false,
  "changed_at": null
}
```

---

## 4. Chats

### 4.1 Create Chat 🔒

Creates a new private (1-to-1) or group chat.

```
POST /chats
```

**Request body — private:**

```json
{
  "type": "private",
  "participant_id": "uuid-of-other-user"
}
```

**Request body — group:**

```json
{
  "type": "group",
  "name": "Project Phoenix",
  "participant_ids": ["uuid1", "uuid2"]
}
```

**Response `201 Created`:**

```json
{
  "id": "chat-uuid",
  "chat_type": "group",
  "name": "Project Phoenix",
  "avatar_url": null,
  "created_by": "creator-uuid",
  "created_at": "2025-04-17T14:00:00Z"
}
```

---

### 4.2 List Chats 🔒

Returns a paginated, cursor-sorted list of chats the authenticated user participates in. Pinned chats appear first.

```
GET /chats?cursor=<cursor>&limit=<limit>
```

| Query param | Type    | Default | Notes                                |
| ----------- | ------- | ------- | ------------------------------------ |
| `cursor`    | string  | —       | Opaque cursor from previous response |
| `limit`     | integer | 20      | Max chats per page                   |

**Response `200 OK`:**

```json
{
  "items": [
    {
      "chat_id": "uuid",
      "chat_type": "private",
      "name": null,
      "avatar_url": null,
      "last_message_id": "msg-uuid",
      "last_message_encrypted": "<base64>",
      "last_sender_id": "uuid",
      "last_message_at": "2025-04-17T13:59:00Z",
      "is_pinned": false,
      "pin_order": 0,
      "is_muted": false,
      "is_archived": false,
      "unread_count": 3
    }
  ],
  "next_cursor": "<opaque>",
  "has_more": true
}
```

---

### 4.3 Get Chat 🔒

Returns full details for a single chat.

```
GET /chats/:id
```

**Path param:** `id` — Chat UUID.

**Response `200 OK`:** Same shape as the item in [Create Chat](#41-create-chat-).

---

### 4.4 Update Chat 🔒

Updates group chat metadata (name, description, avatar). Only admins/owners can update.

```
PATCH /chats/:id
```

**Request body (all fields optional):**

```json
{
  "name": "New Group Name",
  "description": "A short bio",
  "avatar_url": "https://cdn.example.com/avatar.jpg"
}
```

**Response `200 OK`:**

```json
{
  "id": "chat-uuid",
  "chat_type": "group",
  "name": "New Group Name",
  "description": "A short bio",
  "avatar_url": "https://cdn.example.com/avatar.jpg",
  "updated_at": "2025-04-17T14:05:00Z"
}
```

---

### 4.5 Delete Chat 🔒

Deletes a chat (owner / creator only).

```
DELETE /chats/:id
```

**Response `200 OK`:**

```json
{ "deleted": true }
```

---

### 4.6 Update Chat Settings 🔒

Updates per-user settings for a chat (mute, pin, archive).

```
PATCH /chats/:id/settings
```

**Request body (all fields optional):**

```json
{
  "is_muted": true,
  "muted_until": "2025-04-18T00:00:00Z",
  "is_pinned": false,
  "pin_order": 0,
  "is_archived": false
}
```

**Response `200 OK`:**

```json
{
  "is_muted": true,
  "muted_until": "2025-04-18T00:00:00Z",
  "is_pinned": false,
  "pin_order": 0,
  "is_archived": false
}
```

---

## 5. Messages

### 5.1 Send Message 🔒

Sends an encrypted message to a chat. Content must be AES/ChaCha20 encrypted client-side.

```
POST /chats/:id/messages
```

**Path param:** `id` — Chat UUID.

**Request body:**

```json
{
  "content_encrypted": "<base64-ciphertext>",
  "content_iv": "<base64-iv>",
  "message_type": "text",
  "reply_to_id": null,
  "is_forwarded": false,
  "metadata": null
}
```

| Field               | Type   | Required | Notes                                                           |
| ------------------- | ------ | -------- | --------------------------------------------------------------- |
| `content_encrypted` | string | ❌       | Base64 AES ciphertext (omit for attachment-only)                |
| `content_iv`        | string | ❌       | Base64 IV used during encryption                                |
| `message_type`      | string | ✅       | `"text"`, `"image"`, `"video"`, `"audio"`, `"file"`, `"system"` |
| `reply_to_id`       | uuid   | ❌       | ID of message being replied to                                  |
| `is_forwarded`      | bool   | ✅       | Set `true` if forwarding from another chat                      |
| `metadata`          | object | ❌       | Arbitrary JSON (attachment refs, link previews…)                |

**Response `201 Created`:**

```json
{
  "id": "msg-uuid",
  "chat_id": "chat-uuid",
  "sender_id": "user-uuid",
  "reply_to_id": null,
  "content_encrypted": "<base64>",
  "content_iv": "<base64>",
  "message_type": "text",
  "metadata": null,
  "is_forwarded": false,
  "created_at": "2025-04-17T14:10:00Z",
  "edited_at": null,
  "deleted_at": null
}
```

> A `new_message` WebSocket event is broadcast to all chat participants.

---

### 5.2 List Messages 🔒

Returns a paginated list of messages in a chat.

```
GET /chats/:id/messages?cursor=<cursor>&limit=<limit>&direction=<direction>
```

| Query param | Type    | Default    | Notes                                      |
| ----------- | ------- | ---------- | ------------------------------------------ |
| `cursor`    | string  | —          | Opaque cursor                              |
| `limit`     | integer | 50         | Max messages per page                      |
| `direction` | string  | `"before"` | `"before"` or `"after"` relative to cursor |

**Response `200 OK`:**

```json
{
  "items": [
    /* array of MessageResponse */
  ],
  "next_cursor": "<opaque>",
  "has_more": false
}
```

---

### 5.3 Edit Message 🔒

Edits the encrypted content of an existing message. Only the original sender may edit.

```
PATCH /chats/:id/messages/:message_id
```

**Request body:**

```json
{
  "content_encrypted": "<new-base64-ciphertext>",
  "content_iv": "<new-base64-iv>"
}
```

**Response `200 OK`:**

```json
{
  "id": "msg-uuid",
  "chat_id": "chat-uuid",
  "content_encrypted": "<base64>",
  "content_iv": "<base64>",
  "edited_at": "2025-04-17T14:15:00Z"
}
```

> A `message_edited` WebSocket event is broadcast.

---

### 5.4 Delete Message 🔒

Soft-deletes a message (sets `deleted_at`). Only the sender or a chat admin may delete.

```
DELETE /chats/:id/messages/:message_id
```

**Response `200 OK`:**

```json
{ "deleted": true }
```

> A `message_deleted` WebSocket event is broadcast.

---

### 5.5 Mark Messages Read 🔒

Marks all messages in a chat up to the given timestamp as read for the authenticated user.

```
POST /chats/:id/messages/read
```

**Request body:**

```json
{ "up_to": "2025-04-17T14:10:00Z" }
```

**Response `200 OK`:**

```json
{ "updated_count": 5 }
```

> A `messages_read` WebSocket event is broadcast to other participants.

---

## 6. Message Reactions

### 6.1 Add Reaction 🔒

Adds an emoji reaction to a message.

```
POST /chats/:id/messages/:message_id/reactions
```

**Request body:**

```json
{ "reaction": "👍" }
```

**Response `201 Created`:**

```json
{
  "id": "reaction-uuid",
  "message_id": "msg-uuid",
  "user_id": "user-uuid",
  "reaction": "👍",
  "created_at": "2025-04-17T14:12:00Z"
}
```

> A `reaction_added` WebSocket event is broadcast.

---

### 6.2 Remove Reaction 🔒

Removes a specific emoji reaction added by the authenticated user.

```
DELETE /chats/:id/messages/:message_id/reactions/:emoji
```

**Path param:** `emoji` — URL-encoded emoji string, e.g. `%F0%9F%91%8D` for 👍.

**Response `200 OK`:**

```json
{ "removed": true }
```

> A `reaction_removed` WebSocket event is broadcast.

---

## 7. Notifications

### 7.1 List Notifications 🔒

Returns a paginated list of notifications for the authenticated user.

```
GET /notifications?cursor=<cursor>&limit=<limit>
```

**Response `200 OK`:**

```json
{
  "items": [
    {
      "id": "notif-uuid",
      "notification_type": "new_message",
      "data": { "chat_id": "uuid", "message_id": "uuid" },
      "is_read": false,
      "read_at": null,
      "created_at": "2025-04-17T14:10:00Z"
    }
  ],
  "next_cursor": null,
  "has_more": false
}
```

---

### 7.2 Mark Notification Read 🔒

Marks a single notification as read.

```
PATCH /notifications/:notification_id
```

**Response `200 OK`:**

```json
{ "updated": true }
```

---

### 7.3 Mark All Notifications Read 🔒

Marks all unread notifications as read.

```
PATCH /notifications/read-all
```

**Response `200 OK`:**

```json
{ "updated_count": 12 }
```

---

### 7.4 Delete Read Notifications 🔒

Permanently deletes all previously-read notifications.

```
DELETE /notifications/read
```

**Response `200 OK`:**

```json
{ "deleted_count": 8 }
```

---

## 8. Attachments

File uploads use a two-step **pre-signed URL** flow to keep binary data off the API server.

### 8.1 Request Upload URL 🔒

Generates a pre-signed S3 upload URL and registers a pending attachment record.

```
POST /attachments/upload-url
```

**Request body:**

```json
{
  "file_type": "image/jpeg",
  "file_size": 204800,
  "chat_id": "chat-uuid",
  "file_name": "photo.jpg"
}
```

| Field       | Type    | Required | Notes                                     |
| ----------- | ------- | -------- | ----------------------------------------- |
| `file_type` | string  | ✅       | MIME type, e.g. `image/jpeg`, `video/mp4` |
| `file_size` | integer | ✅       | Size in bytes                             |
| `chat_id`   | uuid    | ❌       | Associated chat, if known                 |
| `file_name` | string  | ❌       | Original filename                         |

**Response `200 OK`:**

```json
{
  "upload_url": "https://s3.amazonaws.com/bucket/...<presigned-params>",
  "file_url": "https://cdn.example.com/attachments/attachment-uuid.jpg",
  "attachment_id": "attachment-uuid",
  "expires_at": "2025-04-17T14:25:00Z"
}
```

**Upload flow:**

1. `PUT` the file binary directly to `upload_url` with the correct `Content-Type` header.
2. Call [Confirm Attachment](#82-confirm-attachment-) with the `attachment_id`.

---

### 8.2 Confirm Attachment 🔒

Links the uploaded attachment to a message after the S3 upload completes.

```
POST /attachments/confirm
```

**Request body:**

```json
{
  "attachment_id": "attachment-uuid",
  "message_id": "msg-uuid",
  "encryption_key_enc": "<base64-encrypted-aes-key>",
  "encryption_iv": "<base64-iv>"
}
```

| Field                | Type   | Required | Notes                                 |
| -------------------- | ------ | -------- | ------------------------------------- |
| `attachment_id`      | uuid   | ✅       | From the upload-url response          |
| `message_id`         | uuid   | ✅       | Message this attachment belongs to    |
| `encryption_key_enc` | string | ❌       | Recipient-encrypted AES key (for E2E) |
| `encryption_iv`      | string | ❌       | IV used to encrypt the file           |

**Response `200 OK`:** `{}` (empty object).

---

## 9. Group Management

All group endpoints require the authenticated user to be a participant. Administrative actions (role changes, ownership transfer) require `admin` or `owner` role.

### 9.1 List Participants 🔒

```
GET /chats/:id/participants
```

**Response `200 OK`:**

```json
{
  "participants": [
    {
      "user_id": "uuid",
      "chat_id": "uuid",
      "role": "owner",
      "encryption_key_enc": "<base64>",
      "added_by": null,
      "joined_at": "2025-04-17T10:00:00Z"
    }
  ],
  "count": 1
}
```

**Roles:** `owner` > `admin` > `moderator` > `member`

---

### 9.2 Add Participant 🔒

Adds a user to a group. Caller must be `admin` or `owner`.

```
POST /chats/:id/participants
```

**Request body:**

```json
{
  "user_id": "uuid-of-new-member",
  "encryption_key_enc": "<base64-group-key-encrypted-for-new-member>"
}
```

**Response `201 Created`:**

```json
{
  "participant": {
    /* ParticipantDetailResponse */
  },
  "key_rotation_required": false
}
```

> If `key_rotation_required` is `true`, immediately call [Rotate Group Key](#95-rotate-group-key-).

---

### 9.3 Remove Participant 🔒

Removes a member from the group. Caller must be `admin` or `owner`.

```
DELETE /chats/:id/participants/:user_id
```

**Response `200 OK`:**

```json
{
  "removed": true,
  "key_rotation_required": true
}
```

> `key_rotation_required` is always `true` — rotate the group key after removing any member.

---

### 9.4 Update Participant Role 🔒

Changes a participant's role. Only `owner` can promote to `admin`.  
Cannot promote to `owner` — use [Transfer Ownership](#96-transfer-ownership-) instead.

```
PATCH /chats/:id/participants/:user_id/role
```

**Request body:**

```json
{ "role": "moderator" }
```

Accepted values: `"member"`, `"moderator"`, `"admin"`

**Response `200 OK`:**

```json
{
  "participant": {
    /* ParticipantDetailResponse */
  }
}
```

---

### 9.5 Rotate Group Key 🔒

Distributes a new group encryption key to all remaining members. Must be called after adding/removing participants.

```
POST /chats/:id/rotate-key
```

**Request body:**

```json
{
  "keys": [
    { "user_id": "uuid1", "encryption_key_enc": "<base64>" },
    { "user_id": "uuid2", "encryption_key_enc": "<base64>" }
  ]
}
```

**Response `200 OK`:**

```json
{ "updated_count": 2 }
```

---

### 9.6 Transfer Ownership 🔒

Transfers group ownership to another member. Caller must be current `owner`.

```
POST /chats/:id/transfer-ownership
```

**Request body:**

```json
{ "new_owner_id": "uuid-of-new-owner" }
```

**Response `200 OK`:**

```json
{ "transferred": true }
```

---

### 9.7 Create Invite Link 🔒

Generates a shareable invite link (slug) for the group. Caller must be `admin` or `owner`.

```
POST /chats/:id/invite-link
```

**Response `200 OK`:**

```json
{ "invite_link": "https://mesty.app/join/abc123xyz" }
```

---

### 9.8 Delete Invite Link 🔒

Revokes the current invite link for the group.

```
DELETE /chats/:id/invite-link
```

**Response `200 OK`:**

```json
{ "deleted": true }
```

---

### 9.9 Join Group by Slug 🔒

Joins a group using an invite link slug.

```
POST /chats/join/:slug
```

**Path param:** `slug` — The alphanumeric slug from the invite link.

**Response `200 OK`:**

```json
{
  "participant": {
    /* ParticipantDetailResponse */
  },
  "key_rotation_required": true
}
```

> After joining, request the group key from an admin (out-of-band or via a dedicated key exchange).

---

## 10. Stories

Stories are ephemeral media posts visible to contacts or a custom audience. They expire after 24 hours.

### 10.1 Create Story 🔒

```
POST /stories
```

**Request body:**

```json
{
  "content_url": "https://cdn.example.com/stories/file.mp4",
  "content_type": "video/mp4",
  "caption": "Weekend vibes 🎉",
  "privacy": "contacts",
  "exceptions": []
}
```

| Field          | Type   | Required | Notes                                                      |
| -------------- | ------ | -------- | ---------------------------------------------------------- |
| `content_url`  | string | ✅       | Public URL of the uploaded media                           |
| `content_type` | string | ✅       | MIME type                                                  |
| `caption`      | string | ❌       | Optional text caption                                      |
| `privacy`      | string | ✅       | `"public"`, `"contacts"`, `"selected"`, `"private"`        |
| `exceptions`   | uuid[] | ❌       | For `"selected"`: allow-list; for `"contacts"`: block-list |

**Response `201 Created`:**

```json
{
  "id": "story-uuid",
  "expires_at": "2025-04-18T14:10:00Z"
}
```

---

### 10.2 List Stories (Feed) 🔒

Returns stories from contacts grouped by user.

```
GET /stories
```

**Response `200 OK`:**

```json
[
  {
    "user_id": "uuid",
    "username": "alice",
    "display_name": "Alice",
    "avatar_url": "https://cdn.example.com/avatars/alice.jpg",
    "stories": [
      {
        "id": "story-uuid",
        "content_url": "https://cdn.example.com/stories/file.mp4",
        "content_type": "video/mp4",
        "caption": "Weekend vibes 🎉",
        "privacy": "contacts",
        "created_at": "2025-04-17T14:10:00Z",
        "expires_at": "2025-04-18T14:10:00Z",
        "has_viewed": false
      }
    ]
  }
]
```

---

### 10.3 List My Stories 🔒

Returns all active stories posted by the authenticated user.

```
GET /stories/my
```

**Response `200 OK`:** Array of `StoryWithUserResponse`.

---

### 10.4 Delete Story 🔒

Deletes a story before it expires. Only the owner can delete.

```
DELETE /stories/:id
```

**Response `204 No Content`**

---

### 10.5 View Story 🔒

Records that the authenticated user has viewed a story.

```
POST /stories/:id/view
```

No request body.

**Response `200 OK`:** `{}`

---

### 10.6 React to Story 🔒

Sends an emoji reaction to a story.

```
POST /stories/:id/react
```

**Request body:**

```json
{ "reaction": "🔥" }
```

**Response `200 OK`:** `{}`

---

### 10.7 Get Story Viewers 🔒

Returns the list of users who viewed a story (owner only).

```
GET /stories/:id/views
```

**Response `200 OK`:**

```json
[
  {
    "viewer_id": "uuid",
    "display_name": "Bob",
    "avatar_url": "https://cdn.example.com/avatars/bob.jpg",
    "reaction": "🔥",
    "viewed_at": "2025-04-17T15:00:00Z"
  }
]
```

---

## 11. Users & Profiles

### 11.1 Search Users 🔒

Searches for users by username or display name.

```
GET /users/search?q=<query>&limit=<limit>
```

| Query param | Type    | Required | Notes                    |
| ----------- | ------- | -------- | ------------------------ |
| `q`         | string  | ✅       | Search term (min 1 char) |
| `limit`     | integer | ❌       | Default 20, max 50       |

**Rate limit:** 30 requests / minute.

**Response `200 OK`:**

```json
[
  {
    "id": "uuid",
    "username": "alice",
    "display_name": "Alice Johnson",
    "avatar_url": "https://cdn.example.com/avatars/alice.jpg"
  }
]
```

---

### 11.2 Get My Profile 🔒

Returns the authenticated user's full profile.

```
GET /users/me/profile
```

**Response `200 OK`:**

```json
{
  "id": "uuid",
  "username": "alice",
  "display_name": "Alice Johnson",
  "bio": "Software engineer 🚀",
  "avatar_url": "https://cdn.example.com/avatars/alice.jpg",
  "status_text": "Building things"
}
```

---

### 11.3 Get User Profile 🔒

Returns another user's public profile. Returns `404` if the user has blocked the caller or vice versa.

```
GET /users/:user_id/profile
```

**Response `200 OK`:** Same shape as [Get My Profile](#112-get-my-profile-).

---

## 12. Contacts

### 12.1 List Contacts 🔒

Returns all contacts saved by the authenticated user.

```
GET /contacts
```

**Response `200 OK`:**

```json
[
  {
    "id": "contact-record-uuid",
    "contact_id": "user-uuid-if-registered",
    "phone": "+15559876543",
    "nickname": "Bob from Work",
    "is_favorite": true,
    "created_at": "2025-04-01T10:00:00Z"
  }
]
```

> `contact_id` is `null` if the phone number is not yet registered in the app.

---

### 12.2 Create Contact 🔒

Adds a contact by phone number.

```
POST /contacts
```

**Request body:**

```json
{
  "phone": "+15559876543",
  "nickname": "Bob from Work"
}
```

**Response `201 Created`:** `ContactResponse` object (see above).

---

### 12.3 Update Contact 🔒

Updates a contact's nickname or favorite status.

```
PATCH /contacts/:contact_id
```

**Path param:** `contact_id` — UUID of the contact record.

**Request body (all optional):**

```json
{
  "nickname": "Bobby",
  "is_favorite": false
}
```

**Response `200 OK`:** Updated `ContactResponse`.

---

### 12.4 Delete Contact 🔒

Removes a contact from the user's contact list.

```
DELETE /contacts/:contact_id
```

**Response `204 No Content`**

---

### 12.5 Sync Contacts 🔒

Privacy-preserving contact discovery. The client sends SHA-256 hashes of E.164 phone numbers; the server returns matches without exposing the plaintext numbers.

```
POST /contacts/sync
```

**Request body:**

```json
{
  "hashes": ["a665a45920422f9d417e4867efdc4fb8a04a1f3fff1fa07e998e86f7f7a27ae3", "..."]
}
```

Constraints: 1–1000 hashes per request.

**Response `200 OK`:**

```json
{
  "matches": [
    {
      "hash": "a665a459...",
      "user_id": "uuid",
      "username": "alice",
      "display_name": "Alice"
    }
  ]
}
```

Non-matching hashes are simply omitted from the response.

---

## 13. User Blocks

### 13.1 Block User 🔒

Prevents the target user from messaging or seeing the caller's profile.

```
POST /blocks/:user_id
```

**Path param:** `user_id` — UUID of the user to block.

**Response `201 Created`:**

```json
{
  "id": "block-uuid",
  "blocked_id": "uuid",
  "created_at": "2025-04-17T14:00:00Z"
}
```

---

### 13.2 Unblock User 🔒

Removes a block.

```
DELETE /blocks/:user_id
```

**Response `204 No Content`**

---

### 13.3 List Blocked Users 🔒

Returns all users blocked by the authenticated user.

```
GET /blocks
```

**Response `200 OK`:**

```json
{
  "blocks": [
    {
      "id": "block-uuid",
      "blocked_id": "uuid",
      "created_at": "2025-04-17T14:00:00Z"
    }
  ]
}
```

---

## 14. WebSocket (Real-time)

### 14.1 Connect

```
GET /ws?token=<access_token>
```

Upgrade to WebSocket using a valid **access token** as a query parameter (not in a header, due to WebSocket browser limitations).

```
wss://your-domain/ws?token=<access_token>
```

On successful connection, the server confirms presence to other participants and delivers any queued events.

---

### 14.2 Client → Server Messages

All messages are JSON with `type` and `payload` fields:

```json
{ "type": "<event_type>", "payload": { ... } }
```

#### `typing_start`

Notifies chat participants that the user started typing.

```json
{ "type": "typing_start", "payload": { "chat_id": "uuid" } }
```

#### `typing_stop`

Notifies chat participants that the user stopped typing.

```json
{ "type": "typing_stop", "payload": { "chat_id": "uuid" } }
```

#### `sync_request`

Requests missed events since a given timestamp (useful after reconnect).

```json
{
  "type": "sync_request",
  "payload": { "since": "2025-04-17T14:00:00Z" }
}
```

---

### 14.3 Server → Client Events

The server pushes JSON events with the same `type` / `payload` envelope.

| Event type         | Description                                                                 |
| ------------------ | --------------------------------------------------------------------------- |
| `new_message`      | A new message was sent in one of the user's chats                           |
| `message_edited`   | An existing message was edited                                              |
| `message_deleted`  | A message was soft-deleted                                                  |
| `reaction_added`   | A reaction was added to a message                                           |
| `reaction_removed` | A reaction was removed                                                      |
| `messages_read`    | Another participant marked messages as read                                 |
| `user_online`      | A contact came online                                                       |
| `user_offline`     | A contact went offline                                                      |
| `typing_start`     | A user started typing in a chat                                             |
| `typing_stop`      | A user stopped typing                                                       |
| `key_changed`      | A contact's encryption key changed (safety number verification recommended) |

**Example — `new_message` payload:**

```json
{
  "type": "new_message",
  "payload": {
    "chat_id": "uuid",
    "message": {
      "id": "msg-uuid",
      "sender_id": "uuid",
      "content_encrypted": "<base64>",
      "content_iv": "<base64>",
      "message_type": "text",
      "created_at": "2025-04-17T14:10:00Z"
    }
  }
}
```

**Example — `typing_start` / `typing_stop` payload:**

```json
{
  "type": "typing_start",
  "payload": { "chat_id": "uuid", "user_id": "uuid" }
}
```

**Example — `key_changed` payload:**

```json
{
  "type": "key_changed",
  "payload": { "user_id": "uuid", "timestamp": "2025-04-17T14:00:00Z" }
}
```

---

## 15. Utility Endpoints

### Health Check

Returns `"OK"` (plain text) when the server is running.

```
GET /health
```

**Response `200 OK`:** `OK`

---

### Prometheus Metrics

Exposes internal metrics in Prometheus text format (scrape endpoint).

```
GET /metrics
```

---

## 16. WebRTC & Calling

The backend facilitates peer-to-peer WebRTC connections (audio and video calls) via WebSocket signaling and provides timed TURN credentials for NAT traversal.

### 16.1 Get TURN Credentials 🔒

Returns short-lived, time-limited credentials for your WebRTC `RTCPeerConnection`'s `iceServers`. Includes public STUN servers and authenticated TURN servers.

```
GET /calls/turn-credentials
```

**Response `200 OK`:**

```json
{
  "ice_servers": [
    {
      "urls": ["stun:stun.l.google.com:19302"]
    },
    {
      "urls": ["turn:turn.example.com:3478"],
      "username": "1734567890:uuid",
      "credential": "<base64-hmac-sha1>"
    }
  ],
  "expires_at": 1734567890
}
```

### 16.2 Calling Signaling Lifecycle (WebSockets)

Calls are negotiated over the `ws` connection. All call signaling actions are sent from the client as commands, and the server relays them as events to the peer.

#### A) Initiating a Call
**Client Setup:** Create an `RTCPeerConnection` with the `iceServers` from the REST endpoint. Capture media tracks, add them to the PC, create an SDP offer, set local description.

**Client → Server (`call:initiate`):**
```json
{
  "type": "call:initiate",
  "payload": {
    "receiver_id": "uuid",
    "call_type": "audio", // or "video"
    "offer": { /* SDP object */ }
  }
}
```
*Note: The server will automatically dispatch a Push Notification (`notification_type: 'call'`) to wake up the receiver's device.*

**Server → Receiver (`call:incoming`):**
```json
{
  "type": "call:incoming",
  "payload": {
    "call_id": "uuid",
    "caller_id": "uuid",
    "call_type": "audio",
    "offer": { /* SDP object */ },
    "timestamp": "2025-04-17T14:10:00Z"
  }
}
```

#### B) Accepting a Call
**Receiver Setup:** Set remote description from the offer, create SDP answer, set local description.

**Receiver → Server (`call:accept`):**
```json
{
  "type": "call:accept",
  "payload": {
    "call_id": "uuid",
    "answer": { /* SDP object */ }
  }
}
```

**Server → Caller (`call:accepted`):**
```json
{
  "type": "call:accepted",
  "payload": {
    "call_id": "uuid",
    "receiver_id": "uuid",
    "answer": { /* SDP object */ },
    "timestamp": "2025-04-17T14:10:05Z"
  }
}
```

#### C) Exchanging ICE Candidates
**Peer → Server (`call:ice-candidate`):**
```json
{
  "type": "call:ice-candidate",
  "payload": {
    "call_id": "uuid",
    "receiver_id": "uuid-of-the-other-peer", 
    "candidate": { /* ICE object */ }
  }
}
```

**Server → Peer (`call:ice-candidate`):**
```json
{
  "type": "call:ice-candidate",
  "payload": {
    "call_id": "uuid",
    "sender_id": "uuid",
    "candidate": { /* ICE object */ },
    "timestamp": "2025-04-17T14:10:02Z"
  }
}
```

#### D) Rejecting / Hanging Up a Call
**Peer → Server (`call:reject` or `call:hangup`):**
```json
{
  "type": "call:reject", // Used before accepting
  "payload": {
    "call_id": "uuid",
    "reason": "busy" // or "rejected"
  }
}

// OR

{
  "type": "call:hangup", // Used during active call
  "payload": {
    "call_id": "uuid"
  }
}
```

**Server → Peer (`call:rejected` or `call:ended`):**
```json
{
  "type": "call:rejected",
  "payload": {
    "call_id": "uuid",
    "receiver_id": "uuid",
    "reason": "busy",
    "timestamp": "2025-04-17T14:10:01Z"
  }
}

// OR

{
  "type": "call:ended",
  "payload": {
    "call_id": "uuid",
    "ended_by": "uuid",
    "status": "ended", // "ended", "missed", "rejected", "busy"
    "timestamp": "2025-04-17T14:15:00Z"
  }
}
```

---

## 17. Error Reference

All errors return a JSON body in at least one of these shapes:

```json
{ "error": "Human-readable error message" }
```

```json
{ "message": "Human-readable message" }
```

| HTTP Status                 | Meaning                                                            |
| --------------------------- | ------------------------------------------------------------------ |
| `400 Bad Request`           | Invalid input, missing required field, OTP invalid                 |
| `401 Unauthorized`          | Missing or expired token                                           |
| `403 Forbidden`             | Authenticated but not authorized for the action                    |
| `404 Not Found`             | Resource does not exist or is hidden (e.g., blocked user)          |
| `409 Conflict`              | Resource already exists (e.g., duplicate contact, already blocked) |
| `422 Unprocessable Entity`  | Validation error                                                   |
| `429 Too Many Requests`     | Rate limit exceeded. Back-off and retry after cooldown.            |
| `500 Internal Server Error` | Server-side fault — report with request ID if visible              |

### Common error bodies

```json
// OTP invalid
{ "message": "Código inválido" }

// Rate limited
{ "message": "Too many requests" }

// Not found / blocked
{ "error": "User not found" }

// Already exists
{ "error": "Contact already exists" }
```

---

_Document generated from source code — always cross-check with the latest backend implementation._
