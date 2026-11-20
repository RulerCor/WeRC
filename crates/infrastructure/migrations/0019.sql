-- Separate received-video cache; existing file/image/audio formats stay intact.
CREATE TABLE received_video_assets (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    observation_id TEXT NOT NULL,
    proof TEXT NOT NULL,
    metadata TEXT NOT NULL,
    size INTEGER NOT NULL CHECK(size BETWEEN 1 AND 16777216),
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    expires_at TEXT NOT NULL,
    UNIQUE(account_id, observation_id)
);
CREATE INDEX received_video_expiry ON received_video_assets(account_id, expires_at);
PRAGMA user_version=19;
