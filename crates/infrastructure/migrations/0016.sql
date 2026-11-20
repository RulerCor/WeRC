-- Links contain only product-local identifiers and an authenticated binding.
-- File bytes/names remain in the existing encrypted, quota-limited asset store.
CREATE TABLE received_image_assets (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    observation_id TEXT NOT NULL,
    asset_id TEXT NOT NULL UNIQUE REFERENCES file_assets(id) ON DELETE CASCADE,
    proof TEXT NOT NULL,
    PRIMARY KEY(account_id,observation_id),
    FOREIGN KEY(account_id,observation_id) REFERENCES received_texts(account_id,id) ON DELETE CASCADE
);
PRAGMA user_version=16;
