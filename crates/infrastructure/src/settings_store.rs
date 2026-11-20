use super::*;
use werc_domain::wechat_settings::SettingsChange;

impl SqliteRepository {
    pub(crate) fn find_settings_change(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Option<SettingsChange>> {
        let c = self.connection.lock().map_err(|_| AppError::internal())?;
        let raw: Option<String> = c
            .query_row(
                "SELECT payload FROM wechat_settings_changes WHERE account_id=?1 AND id=?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        raw.map(|v| serde_json::from_str(&v).map_err(|_| AppError::internal()))
            .transpose()
    }
    pub(crate) fn find_settings_key(
        &self,
        account: &str,
        creator: &str,
        digest: &str,
    ) -> AppResult<Option<SettingsChange>> {
        let c = self.connection.lock().map_err(|_| AppError::internal())?;
        let raw:Option<String>=c.query_row("SELECT payload FROM wechat_settings_changes WHERE account_id=?1 AND creator_id=?2 AND idempotency_digest=?3",params![account,creator,digest],|r|r.get(0)).optional().map_err(|_|AppError::internal())?;
        raw.map(|v| serde_json::from_str(&v).map_err(|_| AppError::internal()))
            .transpose()
    }
    pub(crate) fn save_settings_change(&self, v: &SettingsChange) -> AppResult<()> {
        let raw = serde_json::to_string(v).map_err(|_| AppError::internal())?;
        self.connection.lock().map_err(|_|AppError::internal())?.execute("INSERT INTO wechat_settings_changes(id,account_id,creator_id,idempotency_digest,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![v.id,v.account_id,v.creator_id,v.idempotency_digest,raw]).map_err(|_|AppError::internal())?;
        Ok(())
    }
    pub(crate) fn page_settings_changes(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<Vec<(i64, SettingsChange)>> {
        let c = self.connection.lock().map_err(|_| AppError::internal())?;
        let mut stmt=c.prepare("SELECT position,payload FROM wechat_settings_changes WHERE account_id=?1 AND (?2 IS NULL OR position<?2) ORDER BY position DESC LIMIT ?3").map_err(|_|AppError::internal())?;
        let rows = stmt
            .query_map(params![account, before, limit], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|_| AppError::internal())?;
        rows.map(|row| {
            let (p, s) = row.map_err(|_| AppError::internal())?;
            Ok((
                p,
                serde_json::from_str(&s).map_err(|_| AppError::internal())?,
            ))
        })
        .collect()
    }
}
