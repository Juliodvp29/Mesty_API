<p align="center">
  <h1 align="center">🔐 Mesty API</h1>
  <p align="center">
    <strong>A production-ready, end-to-end encrypted messenger backend built in Rust.</strong>
  </p>
  <p align="center">
    <a href="https://github.com/Juliodvp29/messenger_backend/actions/workflows/ci.yml"><img src="https://github.com/Juliodvp29/messenger_backend/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <img src="https://img.shields.io/badge/rust-nightly%20|%20edition%202026-orange?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License">
    <img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg" alt="PRs Welcome">
  </p>
</p>

---

Mesty is a modular, high-performance backend for real-time messaging applications (WhatsApp/Telegram-style). It implements the **X3DH key agreement protocol** for end-to-end encryption, ensuring the server operates as a **zero-knowledge relay** — it never sees plaintext messages.

## Table of Contents

- [Features](#features)
- [Architecture](#architecture)
- [Tech Stack](#tech-stack)
- [Quick Start](#quick-start)
- [API Reference](#api-reference)
- [Security Model](#security-model)
- [Performance & Caching](#performance--caching)
- [Deployment](#deployment)
- [CI/CD](#cicd)
- [Development](#development)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

## Features

- 🔑 **End-to-End Encryption** — X3DH key exchange with per-message AES-GCM encryption
- ⚡ **Real-Time Communication** — WebSocket with presence, typing indicators, and live sync
- 📞 **Voice & Video Calling (WebRTC)** — Secure, low-latency peer-to-peer audio/video calls with real-time WebSocket signaling and dynamic TURN/STUN credentials
- 📱 **Multi-Device Support** — Session management across multiple devices per user
- 📸 **Ephemeral Stories** — 24-hour stories with granular privacy controls and auto-cleanup
- 👥 **Groups & Channels** — Hierarchical roles (owner/admin/moderator/member), invite links, key rotation
- 🔔 **Push Notifications** — FCM (Android/Web) and APNs (iOS) with privacy-safe payloads
- 📇 **Privacy-Preserving Contact Sync** — SHA-256 hashed phone numbers, never sent in plaintext
- 🚦 **Rate Limiting** — Sliding window with Lua-scripted atomic Redis operations
- 📊 **Observability** — Prometheus metrics, structured JSON logging, request tracing
- 🐳 **Production-Ready** — Multi-stage Docker build, Caddy auto-TLS, docker-compose orchestration

## Architecture

Mesty uses a **Cargo Workspace** with clean dependency boundaries enforced across four crates:

```mermaid
graph TB
    API["🌐 api<br/><small>Axum handlers, routing,<br/>middleware, WebSocket</small>"]
    DOMAIN["📦 domain<br/><small>Entities, repositories<br/>(trait definitions), business rules</small>"]
    INFRA["🔧 infrastructure<br/><small>PostgreSQL, Redis, MinIO,<br/>SMTP implementations</small>"]
    SHARED["🧰 shared<br/><small>Config, error types,<br/>cross-cutting utilities</small>"]

    API --> DOMAIN
    API --> INFRA
    API --> SHARED
    DOMAIN --> SHARED
    INFRA --> DOMAIN
    INFRA --> SHARED

    style API fill:#4a9eff,stroke:#333,color:#fff
    style DOMAIN fill:#ff6b6b,stroke:#333,color:#fff
    style INFRA fill:#51cf66,stroke:#333,color:#fff
    style SHARED fill:#ffd43b,stroke:#333,color:#000
```

```
crates/
├── api/                          # HTTP layer
│   ├── handlers/                 # auth, chats, messages, stories, groups, keys, ws, ...
│   ├── middleware/                # JWT auth, rate limiting, request logging, security headers
│   ├── services/                 # jwt, otp, push notifications, S3 storage, websocket
│   ├── routes.rs                 # Route tree definition
│   └── main.rs                   # Application bootstrap
├── domain/                       # Pure business logic (no infra dependencies)
│   ├── chat/                     # Chat & message entities, repository traits
│   ├── stories/                  # Story entities, privacy rules
│   ├── user/                     # User entity, value objects (PhoneNumber, UserId)
│   ├── contact/                  # Contact entity and sync logic
│   └── keys/                     # E2E key models (X3DH protocol)
├── infrastructure/               # External service adapters
│   ├── repositories/             # PostgreSQL implementations of domain traits
│   ├── db/                       # Connection pool management
│   ├── redis/                    # Client, Pub/Sub, caching
│   └── email/                    # SMTP via Lettre
└── shared/                       # Cross-cutting concerns
    ├── config.rs                 # Typed configuration from environment
    ├── error.rs                  # Centralized AppError (thiserror)
    └── logging.rs                # Structured tracing setup
```

## Tech Stack

| Layer               | Technology                   | Purpose                                           |
| ------------------- | ---------------------------- | ------------------------------------------------- |
| **Language**        | Rust (edition 2026, nightly) | Memory safety, performance, fearless concurrency  |
| **Web Framework**   | Axum 0.7                     | Async HTTP, WebSocket, middleware via Tower       |
| **Database**        | PostgreSQL 17 + SQLx 0.8     | Type-safe queries, compile-time checked SQL       |
| **Cache & Pub/Sub** | Redis 8                      | Session cache, presence, rate limiting, event bus |
| **Object Storage**  | MinIO (S3-compatible)        | Attachments, story media, pre-signed URLs         |
| **Reverse Proxy**   | Caddy 2                      | Automatic HTTPS/TLS, routing, load balancing      |
| **Email**           | Lettre + SMTP                | OTP delivery, notifications                       |
| **Auth**            | Ed25519 JWT + TOTP           | Asymmetric token signing, two-factor auth         |
| **Metrics**         | Prometheus                   | HTTP latency, connection pools, business metrics  |
| **CI/CD**           | GitHub Actions               | Automated fmt, clippy, audit, tests               |

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [Rust](https://rustup.rs/) (nightly toolchain)
- [SQLx CLI](https://crates.io/crates/sqlx-cli) — `cargo install sqlx-cli --no-default-features --features postgres`

### Setup

```bash
# 1. Clone the repository
git clone https://github.com/Juliodvp29/messenger_backend.git
cd messenger_backend

# 2. Copy and configure environment variables
cp .env.example .env

# 3. Start infrastructure services
make dev

# 4. Run database migrations
sqlx migrate run --database-url postgres://messenger:messenger_secret@localhost:5434/messenger_dev

# 5. Start the API server
cargo run -p api
```

The API will be available at `http://localhost:3000`. Verify with:

```bash
curl http://localhost:3000/health
```

### Local Services

| Service       | Port   | Description                   |
| ------------- | ------ | ----------------------------- |
| PostgreSQL    | `5434` | Primary database              |
| Redis         | `6379` | Cache, Pub/Sub, rate limiting |
| MinIO API     | `9000` | S3-compatible object storage  |
| MinIO Console | `9001` | Web UI for file management    |
| Mailhog SMTP  | `1025` | Development email server      |
| Mailhog UI    | `8025` | View captured emails          |

### Makefile Commands

```bash
make dev       # Start all infrastructure services
make stop      # Stop all services
make migrate   # Run pending database migrations
make test      # Run all workspace tests
make lint      # Check formatting and run clippy
make build     # Compile release binary
```

## API Reference

All endpoints use JSON (`Content-Type: application/json`). Authenticated endpoints require a `Bearer` token in the `Authorization` header.

### Authentication

| Method   | Endpoint                 | Description                    | Auth   |
| -------- | ------------------------ | ------------------------------ | ------ |
| `POST`   | `/auth/register`         | Send registration OTP          | No     |
| `POST`   | `/auth/verify-phone`     | Verify OTP and create user     | No     |
| `POST`   | `/auth/login`            | Send login OTP                 | No     |
| `POST`   | `/auth/login/verify`     | Verify OTP and receive tokens  | No     |
| `POST`   | `/auth/refresh`          | Renew access token             | No     |
| `POST`   | `/auth/logout`           | End current session            | Bearer |
| `GET`    | `/auth/sessions`         | List active sessions           | Bearer |
| `DELETE` | `/auth/sessions/:id`     | Revoke specific session        | Bearer |
| `POST`   | `/auth/recover`          | Send recovery OTP              | No     |
| `POST`   | `/auth/recover/verify`   | Verify recovery and get tokens | No     |
| `POST`   | `/auth/2fa/setup`        | Initialize TOTP 2FA setup      | Bearer |
| `POST`   | `/auth/2fa/setup/verify` | Confirm TOTP code              | Bearer |
| `POST`   | `/auth/2fa/verify`       | Verify TOTP after login        | No     |
| `GET`    | `/health`                | Health check                   | No     |

<details>
<summary><strong>📋 Auth Flow Examples</strong></summary>

#### Registration

```bash
# 1. Request OTP
curl -X POST http://localhost:3000/auth/register \
  -H "Content-Type: application/json" \
  -d '{"phone":"+573001234567","device_id":"uuid","device_name":"MyPhone","device_type":"android"}'

# 2. Retrieve OTP from Redis (development only)
docker exec messenger_backend-redis-1 redis-cli GET "otp:register:+573001234567"

# 3. Verify OTP and complete registration
curl -X POST http://localhost:3000/auth/verify-phone \
  -H "Content-Type: application/json" \
  -d '{"phone":"+573001234567","code":"123456","device_id":"uuid","device_name":"MyPhone","device_type":"android"}'
# → { "access_token": "...", "refresh_token": "...", "expires_in": 3600, "user": {...} }
```

#### Login

```bash
# 1. Request OTP
curl -X POST http://localhost:3000/auth/login \
  -H "Content-Type: application/json" \
  -d '{"phone":"+573001234567","device_id":"uuid","device_name":"MyPhone","device_type":"android"}'

# 2. Verify OTP
curl -X POST http://localhost:3000/auth/login/verify \
  -H "Content-Type: application/json" \
  -d '{"phone":"+573001234567","code":"123456","device_id":"uuid","device_name":"MyPhone","device_type":"android"}'
```

</details>

### E2E Encryption Keys (X3DH)

| Method | Endpoint                     | Description                         | Auth   |
| ------ | ---------------------------- | ----------------------------------- | ------ |
| `POST` | `/keys/upload`               | Upload Identity Key + Signed Prekey | Bearer |
| `POST` | `/keys/upload-prekeys`       | Upload batch of One-Time Prekeys    | Bearer |
| `GET`  | `/keys/me/count`             | Get available OPK count             | Bearer |
| `GET`  | `/keys/:user_id`             | Fetch user's public key bundle      | Bearer |
| `GET`  | `/keys/:user_id/fingerprint` | Get Identity Key fingerprint        | Bearer |

<details>
<summary><strong>📋 Key Exchange Examples</strong></summary>

```bash
# 1. Upload Identity Key and Signed Prekey (after registration)
curl -X POST http://localhost:3000/keys/upload \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"identity_key":"IK_BASE64","signed_prekey":{"id":1,"key":"SPK_BASE64","signature":"SIG_BASE64"}}'

# 2. Upload One-Time Prekeys (batch of 50)
curl -X POST http://localhost:3000/keys/upload-prekeys \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '[{"id":1,"key":"OPK1_BASE64"},{"id":2,"key":"OPK2_BASE64"}]'

# 3. Fetch recipient's key bundle to initiate a chat
curl http://localhost:3000/keys/{recipient_user_id} \
  -H "Authorization: Bearer $TOKEN"
```

</details>

### Chats & Messages

| Method   | Endpoint                                       | Description                      | Auth   |
| -------- | ---------------------------------------------- | -------------------------------- | ------ |
| `POST`   | `/chats`                                       | Create private or group chat     | Bearer |
| `GET`    | `/chats`                                       | List user's chats (paginated)    | Bearer |
| `GET`    | `/chats/:id`                                   | Get chat by ID                   | Bearer |
| `PATCH`  | `/chats/:id`                                   | Update chat (name, avatar)       | Bearer |
| `DELETE` | `/chats/:id`                                   | Delete chat                      | Bearer |
| `POST`   | `/chats/:id/messages`                          | Send encrypted message           | Bearer |
| `GET`    | `/chats/:id/messages`                          | List messages (cursor-paginated) | Bearer |
| `PATCH`  | `/chats/:id/messages/:msg_id`                  | Edit message                     | Bearer |
| `DELETE` | `/chats/:id/messages/:msg_id`                  | Delete message                   | Bearer |
| `POST`   | `/chats/:id/messages/read`                     | Mark messages as read            | Bearer |
| `POST`   | `/chats/:id/messages/:msg_id/reactions`        | Add reaction                     | Bearer |
| `DELETE` | `/chats/:id/messages/:msg_id/reactions/:emoji` | Remove reaction                  | Bearer |
| `POST`   | `/attachments/upload-url`                      | Generate pre-signed upload URL   | Bearer |
| `POST`   | `/attachments/confirm`                         | Confirm uploaded attachment      | Bearer |

<details>
<summary><strong>📋 Messaging Examples</strong></summary>

```bash
# Send an E2E encrypted message
curl -X POST http://localhost:3000/chats/{chat_id}/messages \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "content_encrypted": "BASE64_AES_GCM_CIPHERTEXT",
    "content_iv": "BASE64_12_BYTES",
    "message_type": "text"
  }'

# Cursor-based message pagination
curl "http://localhost:3000/chats/{chat_id}/messages?limit=50" \
  -H "Authorization: Bearer $TOKEN"

# Load older messages
curl "http://localhost:3000/chats/{chat_id}/messages?cursor={cursor}&direction=before&limit=50" \
  -H "Authorization: Bearer $TOKEN"
```

</details>

### Real-Time (WebSocket)

| Method | Endpoint          | Description          | Auth              |
| ------ | ----------------- | -------------------- | ----------------- |
| `GET`  | `/ws?token={jwt}` | Upgrade to WebSocket | JWT (query param) |

**Server → Client Events:**

| Event                          | Description                      |
| ------------------------------ | -------------------------------- |
| `new_message`                  | A new message was sent in a chat |
| `typing_start` / `typing_stop` | User started/stopped typing      |
| `messages_read`                | Messages were marked as read     |
| `user_online` / `user_offline` | Presence status change           |

**Client → Server Messages:**

```json
{ "type": "typing_start", "payload": { "chat_id": "..." } }
{ "type": "typing_stop",  "payload": { "chat_id": "..." } }
{ "type": "sync_request", "payload": { "since": "2026-01-15T10:30:00Z" } }
```

**Redis Pub/Sub Channels:**

| Channel                 | Purpose                            |
| ----------------------- | ---------------------------------- |
| `user:{user_id}:events` | User-targeted events               |
| `chat:{chat_id}:events` | Chat-scoped events                 |
| `presence:{user_id}`    | Online/offline presence (TTL: 65s) |

### WebRTC Voice & Video Calls

Mesty integrates full support for voice and video calls using WebRTC for low-latency peer-to-peer streaming, with the backend acting as a signaling broker and dynamic TURN/STUN credential generator.

| Method | Endpoint                    | Description                                                | Auth   |
| ------ | --------------------------- | ---------------------------------------------------------- | ------ |
| `GET`  | `/calls/turn-credentials`   | Retrieve dynamic time-limited STUN/TURN server credentials | Bearer |

**WebSocket Signaling Events (Client → Server):**

* `call:initiate` — Initiate a call with an SDP offer.
* `call:accept` — Accept an incoming call with an SDP answer.
* `call:reject` — Decline a call (optionally providing a "busy" or "rejected" reason).
* `call:ice-candidate` — Relay WebRTC ICE candidates directly to the peer.
* `call:hangup` — End an active or unanswered call.

**WebSocket Events (Server → Client):**

* `call:incoming` — Notifies the receiver of an incoming call with the caller's SDP offer.
* `call:accepted` — Relays the receiver's SDP answer back to the caller.
* `call:rejected` — Relays the receiver's rejection/busy reason to the caller.
* `call:ice-candidate` — Relays a remote ICE candidate to establish the peer-to-peer connection.
* `call:ended` — Signals the termination of the call (ended, missed, rejected, busy).

### Stories

| Method   | Endpoint             | Description            | Auth   |
| -------- | -------------------- | ---------------------- | ------ |
| `POST`   | `/stories`           | Publish story          | Bearer |
| `GET`    | `/stories`           | List contacts' stories | Bearer |
| `GET`    | `/stories/my`        | List own stories       | Bearer |
| `DELETE` | `/stories/:id`       | Delete story           | Bearer |
| `POST`   | `/stories/:id/view`  | Register view          | Bearer |
| `POST`   | `/stories/:id/react` | React to story         | Bearer |
| `GET`    | `/stories/:id/views` | View list              | Bearer |

> [!NOTE]
> Story media files (under `attachments/stories/`) are **automatically deleted** from S3/MinIO after 24 hours via lifecycle rules.

**Privacy Modes:**

| Mode              | Visibility                     |
| ----------------- | ------------------------------ |
| `everyone`        | All users                      |
| `contacts`        | Contacts only                  |
| `contacts_except` | Contacts minus specified users |
| `selected`        | Only specified users           |
| `only_me`         | Private (only you)             |

### Notifications

| Method   | Endpoint                  | Description                         | Auth   |
| -------- | ------------------------- | ----------------------------------- | ------ |
| `GET`    | `/notifications`          | List notifications                  | Bearer |
| `PATCH`  | `/notifications/:id`      | Mark as read                        | Bearer |
| `PATCH`  | `/notifications/read-all` | Mark all as read                    | Bearer |
| `DELETE` | `/notifications/read`     | Delete read notifications           | Bearer |
| `PATCH`  | `/chats/:id/settings`     | Configure chat (mute, pin, archive) | Bearer |

### Groups & Channels

| Method   | Endpoint                                | Description                | Auth   |
| -------- | --------------------------------------- | -------------------------- | ------ |
| `GET`    | `/chats/:id/participants`               | List group members         | Bearer |
| `POST`   | `/chats/:id/participants`               | Add member                 | Bearer |
| `DELETE` | `/chats/:id/participants/:user_id`      | Remove member              | Bearer |
| `PATCH`  | `/chats/:id/participants/:user_id/role` | Change role                | Bearer |
| `POST`   | `/chats/:id/invite-link`                | Create/refresh invite link | Bearer |
| `DELETE` | `/chats/:id/invite-link`                | Revoke invite link         | Bearer |
| `POST`   | `/chats/join/:slug`                     | Join via invite link       | Bearer |
| `POST`   | `/chats/:id/rotate-key`                 | Rotate group E2E key       | Bearer |
| `POST`   | `/chats/:id/transfer-ownership`         | Transfer group ownership   | Bearer |

**Participant Roles:** `owner` → `admin` → `moderator` → `member`

> [!NOTE]
> When a new member joins an E2E encrypted group via invite link, the server returns `key_rotation_required: true`. An admin must call `/rotate-key` to securely provision the group key.

### Search & Contacts

| Method   | Endpoint             | Description                     | Auth   |
| -------- | -------------------- | ------------------------------- | ------ |
| `GET`    | `/users/search`      | Global search by username/phone | Bearer |
| `GET`    | `/users/me/profile`  | Get own profile                 | Bearer |
| `GET`    | `/users/:id/profile` | Get public profile              | Bearer |
| `GET`    | `/contacts`          | List contact book               | Bearer |
| `POST`   | `/contacts`          | Add contact                     | Bearer |
| `PATCH`  | `/contacts/:id`      | Edit contact name               | Bearer |
| `DELETE` | `/contacts/:id`      | Remove contact                  | Bearer |
| `POST`   | `/contacts/sync`     | Privacy-preserving bulk sync    | Bearer |
| `GET`    | `/blocks`            | List blocked users              | Bearer |
| `POST`   | `/blocks/:user_id`   | Block user                      | Bearer |
| `DELETE` | `/blocks/:user_id`   | Unblock user                    | Bearer |

### Observability

| Method | Endpoint   | Description                 | Auth |
| ------ | ---------- | --------------------------- | ---- |
| `GET`  | `/metrics` | Prometheus metrics exporter | No   |

## Security Model

Mesty is designed with a **zero-trust** approach to user data:

| Aspect               | Implementation                                                                                                |
| -------------------- | ------------------------------------------------------------------------------------------------------------- |
| **Message Privacy**  | Server is a zero-knowledge relay — all messages are E2E encrypted (AES-GCM). The server never sees plaintext. |
| **Authentication**   | OTP-based (phone verification) + optional local PIN. No passwords stored server-side.                         |
| **Token Signing**    | JWT with **Ed25519** asymmetric keys (not HMAC). Supports key rotation.                                       |
| **Two-Factor Auth**  | TOTP with backup codes via `totp-rs`.                                                                         |
| **Session Security** | Refresh token rotation (one-time use). Multi-device session management.                                       |
| **Rate Limiting**    | Sliding window via Redis Lua scripts. Per-endpoint configurable limits.                                       |
| **Contact Sync**     | Phone numbers are SHA-256 hashed with a global salt before transmission.                                      |
| **HTTP Hardening**   | Security headers: `X-Content-Type-Options`, `X-Frame-Options`, `Strict-Transport-Security`, `X-Request-Id`.   |
| **Push Privacy**     | Push notification payloads **never** include message content.                                                 |

## Performance & Caching

### Redis Roles

Redis serves 7 distinct roles in the system:

| #   | Key Pattern             | Purpose                           | TTL    |
| --- | ----------------------- | --------------------------------- | ------ |
| 1   | `profile:{user_id}`     | Public profile cache              | 5 min  |
| 2   | `session:{session_id}`  | Active session validation         | 15 min |
| 3   | `presence:{user_id}`    | Online/offline status (WebSocket) | 65s    |
| 4   | `rate:{endpoint}:{id}`  | Rate limiting (sliding window)    | 60s    |
| 5   | `user:{user_id}:events` | Pub/Sub event distribution        | —      |
| 6   | `push:queue`            | Push notification job queue       | —      |
| 7   | `refresh:{hash}`        | Refresh token rotation tracking   | 7 days |

### Rate Limits

| Endpoint                        | Limit   | Window |
| ------------------------------- | ------- | ------ |
| `/auth/login`, `/auth/register` | 10 req  | 60s    |
| `/users/search`                 | 30 req  | 60s    |
| Message sending                 | 100 req | 60s    |
| File uploads                    | 20 req  | 60s    |
| General API                     | 300 req | 60s    |

Response headers: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`, `Retry-After` (on 429).

### PostgreSQL Tuning

- **Autovacuum** tuned for high-write tables (`messages`, `chat_participants`, `notifications`)
- **Key indexes**: `idx_messages_chat_created` (message pagination), `idx_users_phone` (phone lookup), `idx_chat_participants_active` (group member queries)

## Deployment

### Development

```bash
# Uses docker-compose.yml with lightweight local services
make dev
cargo run -p api
```

### Production

Mesty ships with a production-ready `docker-compose.prod.yml` that orchestrates the full stack:

```mermaid
graph LR
    CLIENT["Client"] --> CADDY["🔒 Caddy<br/><small>Auto-TLS / HTTPS</small>"]
    CADDY -->|"/*"| API["⚡ Mesty API<br/><small>:3000</small>"]
    CADDY -->|"/s3/*"| MINIO["📦 MinIO<br/><small>:9000</small>"]
    API --> PG["🐘 PostgreSQL<br/><small>:5432</small>"]
    API --> REDIS["🔴 Redis<br/><small>:6379</small>"]
    API --> MINIO
    MIGRATOR["🔄 Migrator<br/><small>ephemeral</small>"] --> PG

    style CADDY fill:#22b8cf,stroke:#333,color:#fff
    style API fill:#4a9eff,stroke:#333,color:#fff
    style PG fill:#336791,stroke:#333,color:#fff
    style REDIS fill:#dc382d,stroke:#333,color:#fff
    style MINIO fill:#c72c48,stroke:#333,color:#fff
    style MIGRATOR fill:#868e96,stroke:#333,color:#fff
```

| Service        | Image                           | Role                                   |
| -------------- | ------------------------------- | -------------------------------------- |
| **PostgreSQL** | `postgres:17-alpine`            | Primary database                       |
| **Redis**      | `redis:8-alpine`                | Cache, Pub/Sub, queues                 |
| **MinIO**      | `minio/minio`                   | S3-compatible object storage           |
| **Caddy**      | `caddy:2-alpine`                | Reverse proxy with automatic HTTPS     |
| **Mesty API**  | Custom (multi-stage Dockerfile) | Application server                     |
| **Migrator**   | `sqlx-cli`                      | Runs migrations on startup (ephemeral) |

#### Deploy

```bash
# 1. Configure production environment
cp .env.production.example .env.production
# Edit .env.production with real credentials (see file for guidance)

# 2. Launch the full stack
docker-compose -f docker-compose.prod.yml up -d

# 3. Verify
curl https://your-domain.com/health
```

> [!IMPORTANT]
> Before deploying, generate fresh **Ed25519 JWT keys** and set strong passwords for PostgreSQL, Redis, and MinIO. See `.env.production.example` for all required variables.

## CI/CD

GitHub Actions runs on every push and PR to `main` and `develop`:

| Job              | Steps                                                                       |
| ---------------- | --------------------------------------------------------------------------- |
| **Code Quality** | `cargo fmt --check` → `cargo clippy -D warnings` → `cargo audit`            |
| **Tests**        | PostgreSQL + Redis services → `sqlx migrate run` → `cargo test --workspace` |

## Development

### Retrieving OTP Codes (Local)

In development, OTP codes are stored in Redis instead of being sent via SMS:

```bash
# After /auth/register
docker exec messenger_backend-redis-1 redis-cli GET "otp:register:+573001234567"

# After /auth/login
docker exec messenger_backend-redis-1 redis-cli GET "otp:login:+573001234567"

# After /auth/recover
docker exec messenger_backend-redis-1 redis-cli GET "otp:recover:+573001234567"
```

### Code Conventions

- **Branch**: `develop` (main development branch)
- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/) (`feat`, `fix`, `chore`, `docs`, `test`, `refactor`)
- **SQL**: Always use `sqlx::query!` / `sqlx::query_as!` for type-safe queries. Never interpolate user input.
- **Dependencies**: `api → domain → infrastructure → shared` (enforced by crate boundaries)
- **Errors**: Centralized `AppError` with `thiserror`. Infra errors → domain errors → HTTP responses.
- **Auth**: JWT middleware on all protected routes. Ed25519 asymmetric signing.

## Roadmap

| Phase | Description                 | Status  |
| ----- | --------------------------- | ------- |
| 01    | Foundations & Configuration | ✅ Done |
| 02    | Database & Migrations       | ✅ Done |
| 03    | Authentication & Security   | ✅ Done |
| 04    | End-to-End Encryption       | ✅ Done |
| 05    | Chats & Messages            | ✅ Done |
| 06    | Real-Time (WebSockets)      | ✅ Done |
| 07    | Stories & Reactions         | ✅ Done |
| 08    | Push Notifications          | ✅ Done |
| 09    | Groups & Channels           | ✅ Done |
| 10    | Search & Contacts           | ✅ Done |
| 11    | Performance & Caching       | ✅ Done |
| 12    | Observability & Hardening   | ✅ Done |
| 13    | Deployment & Production     | ✅ Done |
| 14    | WebRTC Voice & Video Calls  | ✅ Done |

## Contributing

Contributions are welcome! Please open an issue first to discuss what you'd like to change.

1. Fork the repository
2. Create your feature branch (`git checkout -b feat/amazing-feature`)
3. Run quality checks (`make lint && make test`)
4. Commit with [Conventional Commits](https://www.conventionalcommits.org/)
5. Push and open a Pull Request

## License

This project is licensed under the [MIT License](LICENSE).

---

<p align="center">
  Built with 🦀 Rust by <a href="https://github.com/Juliodvp29">Julio Otero</a>
</p>
