CREATE TABLE setup_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    payload BLOB,
    dismissed INTEGER NOT NULL DEFAULT 0 CHECK(dismissed IN (0,1))
);
INSERT INTO setup_state(singleton) VALUES(1);
PRAGMA user_version=6;
