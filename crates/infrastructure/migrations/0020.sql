-- Optional approval timing; legacy immutable draft ciphertext is unchanged.
CREATE TABLE IF NOT EXISTS send_schedules (
    outbox_id TEXT PRIMARY KEY REFERENCES send_outbox(id) ON DELETE CASCADE,
    scheduled_at INTEGER NOT NULL CHECK(scheduled_at > 0)
);
CREATE INDEX IF NOT EXISTS send_schedules_due ON send_schedules(scheduled_at);
PRAGMA user_version = 20;
