-- A source is an isolation scope, not an authenticated WeChat identity.
-- Legacy singleton accounts are deliberately not inferred into this mapping.
CREATE TABLE setup_source_accounts (
    source_fingerprint TEXT PRIMARY KEY CHECK(length(source_fingerprint)=64),
    account_id TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE
);
PRAGMA user_version=7;
