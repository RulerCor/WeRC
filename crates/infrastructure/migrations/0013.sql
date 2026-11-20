-- Product-only durable source windows. No join/alias with native hook IDs.
CREATE TABLE receive_recovery_windows (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    summary TEXT NOT NULL,
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    UNIQUE(account_id,id)
);
CREATE INDEX receive_recovery_page ON receive_recovery_windows(account_id,position);
PRAGMA user_version=13;
