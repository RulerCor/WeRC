use crate::*;

// VACUUM INTO 使用 SQLite 一致快照（包含已提交 WAL），不裸拷主文件。
// 只备份本产品状态库；不对微信来源运行此操作。
pub(crate) fn before_upgrade(connection: &Connection, path: &Path) -> AppResult<()> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| AppError::internal())?;
    if !(1..22).contains(&version) {
        return Ok(());
    }
    let boundary = if version < 9 { 9 } else { version + 1 };
    let destination = path.with_file_name(format!(
        "{}.pre-v{}-{}.sqlite3",
        path.file_name().unwrap_or_default().to_string_lossy(),
        boundary,
        Uuid::new_v4()
    ));
    connection
        .execute(
            "VACUUM main INTO ?1",
            [destination.to_str().ok_or_else(AppError::internal)?],
        )
        .map_err(|_| {
            AppError::new(
                "MIGRATION_BACKUP_FAILED",
                "升级前一致备份失败，未开始结构升级",
            )
        })?;
    let backup =
        Connection::open_with_flags(&destination, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| {
                AppError::new("MIGRATION_BACKUP_FAILED", "备份回读失败，未开始结构升级")
            })?;
    let integrity: String = backup
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|_| AppError::internal())?;
    let saved_version: i64 = backup
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| AppError::internal())?;
    if integrity != "ok" || saved_version != version {
        return Err(AppError::new(
            "MIGRATION_BACKUP_FAILED",
            "备份完整性或结构版本校验失败，未开始结构升级",
        ));
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(destination)
        .and_then(|f| f.sync_all())
        .map_err(|_| AppError::new("MIGRATION_BACKUP_FAILED", "备份同步失败，未开始结构升级"))?;
    Ok(())
}
