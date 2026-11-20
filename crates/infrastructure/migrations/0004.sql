CREATE TABLE operations (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    state TEXT NOT NULL,
    record TEXT NOT NULL,
    UNIQUE(account_id, creator_id, idempotency_key)
);
CREATE INDEX operations_pending ON operations(state);
PRAGMA user_version = 4;
