-- Product-owned file contents and names, always encrypted at rest.
CREATE TABLE file_assets (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    size INTEGER NOT NULL,
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX file_assets_account ON file_assets(account_id, expires_at);
PRAGMA user_version=15;
