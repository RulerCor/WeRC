use crate::*;
#[derive(serde::Serialize)]
pub struct StorageUsage {
    pub database_bytes: u64,
    pub reusable_bytes: u64,
    pub query_bytes: u64,
    pub query_rows: u64,
    pub protected_records: u64,
}
impl SqliteRepository {
    /// Validate a closed product-store copy before switching its locator. This
    /// does not run migrations, open external WeChat DBs, or emit plaintext.
    pub fn verify_storage_copy(path: &std::path::Path) -> AppResult<()> {
        let c =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| AppError::new("STORAGE_VERIFY", "迁移副本无法打开"))?;
        let integrity: String = c
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|_| AppError::internal())?;
        if integrity != "ok" {
            return Err(AppError::new("STORAGE_VERIFY", "迁移副本完整性检查未通过"));
        }
        Ok(())
    }

    pub fn storage_usage(&self) -> AppResult<StorageUsage> {
        let c = self.connection()?;
        let scalar = |sql: &str| {
            c.query_row(sql, [], |r| r.get::<_, u64>(0))
                .map_err(|_| AppError::internal())
        };
        let page = scalar("PRAGMA page_size")?;
        Ok(StorageUsage {
            database_bytes: scalar("PRAGMA page_count")?.saturating_mul(page),
            reusable_bytes: scalar("PRAGMA freelist_count")?.saturating_mul(page),
            query_bytes: scalar(
                "SELECT coalesce((SELECT sum(length(ciphertext)+length(nonce)+length(wrapped_key)) FROM history_projections),0)+coalesce((SELECT sum(length(ciphertext)+length(nonce)) FROM history_projection_rows),0)",
            )?,
            query_rows: scalar("SELECT count(*) FROM history_projection_rows")?,
            protected_records: scalar(
                "SELECT (SELECT count(*) FROM library_documents)+(SELECT count(*) FROM received_texts)",
            )?,
        })
    }
}
