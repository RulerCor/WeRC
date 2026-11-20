CREATE TABLE audio_assets (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    size INTEGER NOT NULL CHECK(size > 0 AND size <= 8388608),
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL CHECK(length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX audio_assets_account ON audio_assets(account_id, expires_at);
PRAGMA user_version = 3;
