-- ============================================================
--  MIGRATION 019: WebRTC Calls Support
--  Adds call_status and call_type ENUMs and the calls table
--  for tracking call history between users.
-- ============================================================

CREATE TYPE call_status AS ENUM (
    'initiated',  -- caller has initiated, not yet ringing on receiver
    'ringing',    -- receiver has been notified (WS or Push delivered)
    'answered',   -- receiver accepted, WebRTC negotiation in progress
    'ended',      -- call terminated normally by either party
    'missed',     -- receiver never answered and the call expired/was hung up
    'rejected',   -- receiver explicitly declined
    'busy'        -- receiver is already in another call
);

CREATE TYPE call_type AS ENUM ('audio', 'video');

-- ============================================================
--  CALLS
--  Stores one record per call attempt.
--  SDP payloads and ICE candidates are NOT persisted here —
--  they are relayed in real-time via WebSocket / Redis pub-sub.
--
--  started_at: set when the receiver accepts (status -> answered)
--  ended_at:   set on hangup, rejection, timeout, or missed
-- ============================================================

CREATE TABLE calls (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    caller_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    receiver_id  UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    type         call_type   NOT NULL,
    status       call_status NOT NULL DEFAULT 'initiated',
    started_at   TIMESTAMPTZ,                -- NULL until answered
    ended_at     TIMESTAMPTZ,                -- NULL while active
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (caller_id <> receiver_id)
);

CREATE TRIGGER calls_updated_at
    BEFORE UPDATE ON calls
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

-- Index for call history queries
CREATE INDEX idx_calls_caller   ON calls (caller_id,   created_at DESC);
CREATE INDEX idx_calls_receiver ON calls (receiver_id, created_at DESC);
-- Quickly find active calls for a user (to detect busy state)
CREATE INDEX idx_calls_active   ON calls (caller_id, receiver_id)
    WHERE status IN ('initiated', 'ringing', 'answered');
