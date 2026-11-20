-- 与仅本地 audio.inspect 分离；正文、真实身份与目标只保存在认证密文中。
CREATE TABLE send_secrets (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    wrapped_key BLOB NOT NULL
);
CREATE TABLE send_outbox (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL,
    idempotency_digest TEXT NOT NULL,
    metadata TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('awaiting_approval','queued','submitting','unknown','server_accepted','recipient_confirmed','rejected','canceled','expired')),
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    approval_expires_at INTEGER,
    approval_proof BLOB,
    attempt_id TEXT,
    evidence TEXT,
    updated_at INTEGER NOT NULL,
    UNIQUE(account_id,creator_id,idempotency_digest)
);
CREATE TABLE send_attempts (
    id TEXT PRIMARY KEY,
    outbox_id TEXT NOT NULL UNIQUE REFERENCES send_outbox(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    recovered INTEGER NOT NULL DEFAULT 0 CHECK(recovered IN (0,1))
);
CREATE TABLE send_receipts (
    outbox_id TEXT NOT NULL REFERENCES send_outbox(id) ON DELETE CASCADE,
    attempt_id TEXT NOT NULL REFERENCES send_attempts(id) ON DELETE CASCADE,
    evidence TEXT NOT NULL,
    protected BLOB NOT NULL,
    observed_at INTEGER NOT NULL,
    PRIMARY KEY(outbox_id,attempt_id,evidence)
);
CREATE INDEX send_outbox_scope ON send_outbox(account_id,updated_at);
PRAGMA user_version=10;
