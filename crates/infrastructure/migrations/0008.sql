ALTER TABLE history_sources ADD COLUMN storage_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE history_sources ADD COLUMN visible INTEGER NOT NULL DEFAULT 1 CHECK(visible IN (0,1));
ALTER TABLE history_sources ADD COLUMN payload_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE history_sources ADD COLUMN batch_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE history_sources ADD COLUMN started_at TEXT NOT NULL DEFAULT '';
CREATE TABLE history_batches (
 source_id TEXT NOT NULL REFERENCES history_sources(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL,
 nonce BLOB NOT NULL CHECK(length(nonce)=12),
 ciphertext BLOB NOT NULL CHECK(length(ciphertext)<=2129168),
 PRIMARY KEY(source_id,sequence)
);
CREATE TABLE history_record_digests (
 source_id TEXT NOT NULL REFERENCES history_sources(id) ON DELETE CASCADE,
 message_id TEXT NOT NULL,
 digest TEXT NOT NULL,
 PRIMARY KEY(source_id,message_id)
);
CREATE INDEX history_record_identity ON history_record_digests(message_id,source_id);
CREATE TABLE history_pending_events (
 source_id TEXT NOT NULL REFERENCES history_sources(id) ON DELETE CASCADE,
 message_id TEXT NOT NULL,
 change TEXT NOT NULL CHECK(change IN ('added','updated')),
 PRIMARY KEY(source_id,message_id)
);
CREATE TABLE history_index_keys (
 account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
 wrapped_key BLOB NOT NULL
);
PRAGMA user_version=8;
