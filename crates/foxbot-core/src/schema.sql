CREATE TABLE IF NOT EXISTS sessions (
    key TEXT PRIMARY KEY, public_ref TEXT NOT NULL UNIQUE, binding TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1, attempts INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY, conversation TEXT NOT NULL REFERENCES sessions(key),
    identity_epoch INTEGER NOT NULL, canonical_id TEXT, payload TEXT NOT NULL, observed_ms INTEGER NOT NULL,
    disposition TEXT NOT NULL, task_id TEXT,
    UNIQUE(conversation, identity_epoch, canonical_id)
);
CREATE TABLE IF NOT EXISTS observations (
    conversation TEXT NOT NULL REFERENCES sessions(key), source TEXT NOT NULL,
    source_id TEXT NOT NULL, message_id TEXT NOT NULL REFERENCES messages(id),
    payload TEXT NOT NULL,
    PRIMARY KEY(conversation, source, source_id)
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY, conversation TEXT NOT NULL REFERENCES sessions(key),
    revision INTEGER NOT NULL, profile_version INTEGER NOT NULL,
    request TEXT NOT NULL, response TEXT, created_ms INTEGER NOT NULL, state TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
    id TEXT PRIMARY KEY, task_id TEXT NOT NULL UNIQUE REFERENCES tasks(id),
    conversation TEXT NOT NULL REFERENCES sessions(key), payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('PREPARED','EXECUTING','SUBMITTED','VERIFIED_OUTGOING','UNKNOWN','STALE','BLOCKED')),
    reason TEXT
);
CREATE TABLE IF NOT EXISTS transitions (
    seq INTEGER PRIMARY KEY AUTOINCREMENT, action_id TEXT NOT NULL REFERENCES outbox(id),
    state TEXT NOT NULL, reason TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_queue ON messages(conversation, disposition, observed_ms);
CREATE INDEX IF NOT EXISTS actions_pending ON outbox(conversation, state);
PRAGMA application_id = 1178753073;
CREATE TABLE IF NOT EXISTS service_exchanges (
    request_id TEXT PRIMARY KEY REFERENCES tasks(id), profile_tag TEXT NOT NULL,
    conversation_ref TEXT NOT NULL, stateful INTEGER NOT NULL, wants_receipts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS service_receipts (
    id TEXT PRIMARY KEY, request_id TEXT NOT NULL REFERENCES service_exchanges(request_id),
    revision INTEGER NOT NULL, payload TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('PENDING','IN_FLIGHT','ACKED','SUSPENDED')),
    attempts INTEGER NOT NULL DEFAULT 0, next_ms INTEGER NOT NULL DEFAULT 0,
    UNIQUE(request_id, revision)
);
CREATE INDEX IF NOT EXISTS service_receipts_due ON service_receipts(status, next_ms);
CREATE TABLE IF NOT EXISTS host_control (
    id INTEGER PRIMARY KEY CHECK(id=1), paused INTEGER NOT NULL CHECK(paused IN (0,1))
);
PRAGMA user_version = 3;
