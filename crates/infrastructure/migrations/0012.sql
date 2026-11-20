-- Product-only encrypted native observations. A checkpoint and its message
-- commit atomically; acknowledgement is sent only after this transaction.
CREATE TABLE receive_streams (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    cursor TEXT NOT NULL
);
CREATE TABLE native_identities (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    identity_token TEXT NOT NULL
);
CREATE TABLE received_texts (
    position INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    wrapped_key BLOB NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    UNIQUE(account_id,id)
);
CREATE INDEX received_texts_page ON received_texts(account_id,position);
CREATE TABLE receive_usage (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    messages INTEGER NOT NULL,
    bytes INTEGER NOT NULL
);
CREATE TRIGGER received_texts_usage AFTER INSERT ON received_texts BEGIN
    INSERT INTO receive_usage(account_id,messages,bytes) VALUES(NEW.account_id,1,length(NEW.ciphertext)+length(NEW.wrapped_key))
    ON CONFLICT(account_id) DO UPDATE SET messages=messages+1,bytes=bytes+length(NEW.ciphertext)+length(NEW.wrapped_key);
END;
PRAGMA user_version=12;
