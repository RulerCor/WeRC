-- 单当前微信。accounts 旧行继续作为密文/归档隔离键，不合并、不重编号。
CREATE TABLE current_connection (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    revision INTEGER NOT NULL CHECK(revision>=0),
    selection_required INTEGER NOT NULL CHECK(selection_required IN (0,1))
);
INSERT INTO current_connection(singleton,account_id,revision,selection_required)
SELECT 1, CASE WHEN COUNT(*)=1 THEN MIN(id) ELSE NULL END,
       CASE WHEN COUNT(*)=1 THEN 1 ELSE 0 END,
       CASE WHEN COUNT(*)>1 THEN 1 ELSE 0 END FROM accounts;
-- 不改写或扩大旧令牌。仅保留唯一当前来源的单范围令牌。
UPDATE tokens SET revoked=1 WHERE
    (SELECT account_id FROM current_connection WHERE singleton=1) IS NULL
    OR (SELECT COUNT(*) FROM token_accounts WHERE token_id=tokens.id)<>1
    OR NOT EXISTS(SELECT 1 FROM token_accounts a JOIN current_connection c
                  ON a.account_id=c.account_id WHERE a.token_id=tokens.id);
CREATE TRIGGER accounts_single_connection BEFORE INSERT ON accounts
WHEN EXISTS(SELECT 1 FROM accounts)
BEGIN SELECT RAISE(ABORT,'SINGLE_CONNECTION_REQUIRED'); END;
CREATE TRIGGER accounts_stable_identity BEFORE UPDATE OF id ON accounts
BEGIN SELECT RAISE(ABORT,'ACCOUNT_ID_IMMUTABLE'); END;
CREATE TRIGGER binding_insert_revision AFTER INSERT ON client_bindings
BEGIN UPDATE current_connection SET revision=revision+1 WHERE account_id=NEW.account_id; END;
CREATE TRIGGER binding_update_revision AFTER UPDATE ON client_bindings
BEGIN UPDATE current_connection SET revision=revision+1 WHERE account_id=NEW.account_id; END;
CREATE TRIGGER binding_delete_revision AFTER DELETE ON client_bindings
BEGIN UPDATE current_connection SET revision=revision+1 WHERE account_id=OLD.account_id; END;
PRAGMA user_version=9;
