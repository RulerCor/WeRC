CREATE TABLE IF NOT EXISTS library_documents (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    UNIQUE(account_id,id)
);
CREATE INDEX IF NOT EXISTS library_documents_listing ON library_documents(account_id,kind,position DESC);
PRAGMA user_version = 21;
