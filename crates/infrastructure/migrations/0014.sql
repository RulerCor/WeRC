-- Product-owned canonical PNG assets, always encrypted at rest.
CREATE TABLE image_assets (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    metadata TEXT NOT NULL,
    size INTEGER NOT NULL,
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX image_assets_account ON image_assets(account_id, expires_at);
PRAGMA user_version=14;
