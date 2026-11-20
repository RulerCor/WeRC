-- Inline voice bytes use existing encrypted file assets. The observation,
-- voice asset/link, and cursor commit together before native acknowledgement.
CREATE TABLE received_voice_assets (
 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
 observation_id TEXT NOT NULL,
 asset_id TEXT NOT NULL UNIQUE REFERENCES file_assets(id) ON DELETE CASCADE,
 proof TEXT NOT NULL,
 PRIMARY KEY(account_id,observation_id),
 FOREIGN KEY(account_id,observation_id) REFERENCES received_texts(account_id,id) ON DELETE CASCADE
);
PRAGMA user_version=17;
