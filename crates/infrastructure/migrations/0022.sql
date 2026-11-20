CREATE TABLE wechat_settings_changes (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL,
    idempotency_digest TEXT NOT NULL,
    payload TEXT NOT NULL,
    UNIQUE(account_id, creator_id, idempotency_digest)
);
CREATE INDEX wechat_settings_changes_page ON wechat_settings_changes(account_id,position);
PRAGMA user_version = 22;
