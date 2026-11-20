CREATE TABLE history_sources (
 id TEXT PRIMARY KEY,
 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
 source_sha256 TEXT NOT NULL,
 metadata TEXT NOT NULL,
 wrapped_key BLOB NOT NULL,
 nonce BLOB NOT NULL CHECK(length(nonce)=12),
 ciphertext BLOB NOT NULL CHECK(length(ciphertext)<=33554448),
 UNIQUE(account_id,source_sha256)
);
CREATE INDEX history_sources_account ON history_sources(account_id);
PRAGMA user_version=5;
