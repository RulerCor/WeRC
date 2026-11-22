use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use werc_domain::{
    AppError, AppResult,
    connection::{ClientBinding, ClientInstallation},
};

impl SqliteRepository {
    pub(crate) fn refresh_binding_preserving_key_store(&self, old: &ClientBinding, new: &ClientBinding, old_key: Option<&[u8]>, new_key: Option<&[u8]>, revision: i64) -> AppResult<()> {
        if old.account_id != new.account_id || old.installation.id != new.installation.id || old_key.is_some() != new_key.is_some() {
            return Err(AppError::new("CONNECTION_CHANGED", "绑定或密钥不匹配"));
        }
        let mut c = self.connection()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, &old.account_id)?;
        let current: Option<(String, String)> = tx.query_row("SELECT installation,bound_at FROM client_bindings WHERE account_id=?1", [&old.account_id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| AppError::internal())?;
        let current = current.map(|(json, bound)| serde_json::from_str::<ClientInstallation>(&json).map(|i| (i,bound))).transpose().map_err(|_| AppError::internal())?;
        let current_key: Option<Vec<u8>> = tx.query_row("SELECT protected FROM protected_keys WHERE account_id=?1", [&old.account_id], |r| r.get(0)).optional().map_err(|_| AppError::internal())?;
        let current_revision: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')", [&old.account_id], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if !current.is_some_and(|(i,bound)| i.id == old.installation.id && i.sha256 == old.installation.sha256 && bound == old.bound_at) || current_key.as_deref() != old_key || current_revision != revision {
            return Err(AppError::new("CONNECTION_CHANGED", "绑定或密钥已变化，请重试"));
        }
        let json = serde_json::to_string(&new.installation).map_err(|_| AppError::internal())?;
        tx.execute("UPDATE client_bindings SET installation=?2,bound_at=?3,checked_at=?4,validation=?5 WHERE account_id=?1", params![new.account_id,json,new.bound_at,new.checked_at,new.validation]).map_err(|_| AppError::internal())?;
        if let Some(key) = new_key {
            tx.execute("UPDATE protected_keys SET protected=?2,updated_at=?3 WHERE account_id=?1", params![new.account_id,key,new.checked_at]).map_err(|_| AppError::internal())?;
        }
        let payload = serde_json::json!({"version":new.installation.version,"fingerprint_changed":true,"key_preserved":new_key.is_some()}).to_string();
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'connection.bound',?2,?3)",params![new.account_id,new.checked_at,payload]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn bindings_store(&self) -> AppResult<Vec<ClientBinding>> {
        let c = self.connection()?;
        let mut statement = c.prepare("SELECT account_id,installation,bound_at,checked_at,validation FROM client_bindings ORDER BY account_id").map_err(|_| AppError::internal())?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| AppError::internal())?;
        rows.map(|row| {
            let (account_id, json, bound_at, checked_at, validation) =
                row.map_err(|_| AppError::internal())?;
            Ok(ClientBinding {
                account_id,
                installation: serde_json::from_str(&json).map_err(|_| AppError::internal())?,
                bound_at,
                checked_at,
                validation,
            })
        })
        .collect()
    }
    pub(crate) fn save_binding_store(&self, binding: &ClientBinding) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, &binding.account_id)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT installation FROM client_bindings WHERE account_id=?1",
                [&binding.account_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let changed = old
            .map(|json| {
                serde_json::from_str::<ClientInstallation>(&json).map(|old| {
                    old.id != binding.installation.id || old.sha256 != binding.installation.sha256
                })
            })
            .transpose()
            .map_err(|_| AppError::internal())?
            .unwrap_or(true);
        if changed {
            tx.execute(
                "DELETE FROM protected_keys WHERE account_id=?1",
                [&binding.account_id],
            )
            .map_err(|_| AppError::internal())?;
        }
        let json =
            serde_json::to_string(&binding.installation).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO client_bindings(account_id,installation,bound_at,checked_at,validation) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account_id) DO UPDATE SET installation=excluded.installation,bound_at=excluded.bound_at,checked_at=excluded.checked_at,validation=excluded.validation", params![binding.account_id,json,binding.bound_at,binding.checked_at,binding.validation]).map_err(|_| AppError::internal())?;
        let payload = serde_json::json!({"version":binding.installation.version,"fingerprint_changed":changed}).to_string();
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'connection.bound',?2,?3)", params![binding.account_id,binding.checked_at,payload]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn remove_binding_store(&self, account: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, account)?;
        tx.execute("DELETE FROM protected_keys WHERE account_id=?1", [account])
            .map_err(|_| AppError::internal())?;
        tx.execute("DELETE FROM client_bindings WHERE account_id=?1", [account])
            .map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'connection.unbound',?2,'{}')", params![account,Utc::now().to_rfc3339()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn key_revision_store(&self, account: &str) -> AppResult<i64> {
        self.connection()?.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[account],|row|row.get(0)).map_err(|_| AppError::internal())
    }
    pub(crate) fn store_key_store(
        &self,
        binding: &ClientBinding,
        protected: &[u8],
        expected_revision: Option<i64>,
    ) -> AppResult<()> {
        let account = &binding.account_id;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, account)?;
        let current: Option<(String, String)> = tx
            .query_row(
                "SELECT installation,bound_at FROM client_bindings WHERE account_id=?1",
                [account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let current: Option<(ClientInstallation, String)> = current
            .map(|(json, bound_at)| {
                serde_json::from_str(&json).map(|installation| (installation, bound_at))
            })
            .transpose()
            .map_err(|_| AppError::internal())?;
        if !current.is_some_and(|(c, bound_at)| {
            c.id == binding.installation.id
                && c.sha256 == binding.installation.sha256
                && bound_at == binding.bound_at
        }) {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "密钥保护期间来源已改变，请重新操作",
            ));
        }
        if let Some(expected) = expected_revision {
            let current:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[account],|row|row.get(0)).map_err(|_| AppError::internal())?;
            if current != expected {
                return Err(AppError::new(
                    "KEY_STATE_CHANGED",
                    "取钥期间密钥或绑定已被清除/替换，本次结果已丢弃",
                ));
            }
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO protected_keys(account_id,protected,updated_at) VALUES (?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET protected=excluded.protected,updated_at=excluded.updated_at", params![account,protected,now]).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'key.protected',?2,'{}')", params![account,now]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn protected_key_store(
        &self,
        account: &str,
    ) -> AppResult<Option<(Vec<u8>, String)>> {
        self.connection()?
            .query_row(
                "SELECT protected,updated_at FROM protected_keys WHERE account_id=?1",
                [account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::internal())
    }
    pub(crate) fn remove_key_store(&self, account: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        tx.execute("DELETE FROM protected_keys WHERE account_id=?1", [account])
            .map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'key.removed',?2,'{}')", params![account,Utc::now().to_rfc3339()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
}
