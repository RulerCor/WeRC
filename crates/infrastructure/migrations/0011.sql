-- 仅产品侧可重建的受保护投影，不修改原历史密文或来源。
CREATE TABLE history_projections (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    generation TEXT NOT NULL,
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    ready INTEGER NOT NULL DEFAULT 0 CHECK(ready IN (0,1)),
    payload_bytes INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX history_projection_ready ON history_projections(account_id,generation) WHERE ready=1;
CREATE TABLE history_projection_rows (
    projection_id TEXT NOT NULL REFERENCES history_projections(id) ON DELETE CASCADE,
    token TEXT NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    PRIMARY KEY(projection_id,token)
);
PRAGMA user_version=11;
