CREATE TABLE client_bindings (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    installation TEXT NOT NULL,
    bound_at TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    validation TEXT NOT NULL
);
CREATE TABLE protected_keys (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    protected BLOB NOT NULL,
    updated_at TEXT NOT NULL
);
PRAGMA user_version = 2;
