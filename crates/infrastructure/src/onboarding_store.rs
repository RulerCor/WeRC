use crate::*;
use rusqlite::TransactionBehavior;

impl SqliteRepository {
    pub(crate) fn bind_current_source_store(
        &self,
        account: &str,
        revision: i64,
        fingerprint: &str,
    ) -> AppResult<()> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("INVALID_ARGUMENT", "来源指纹无效"));
        }
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let valid:bool=tx.query_row("SELECT account_id=?1 AND revision=?2 AND selection_required=0 FROM current_connection WHERE singleton=1",params![account,revision],|r|r.get::<_,Option<bool>>(0)).map_err(|_|AppError::internal())?.unwrap_or(false);
        if !valid {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "当前微信或修订号已变化",
            ));
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM setup_source_accounts WHERE (account_id=?1 AND source_fingerprint<>?2) OR (source_fingerprint=?2 AND account_id<>?1))",params![account,fingerprint],|r|r.get(0)).map_err(|_|AppError::internal())?;
        if conflict {
            return Err(AppError::new(
                "SOURCE_CONFLICT",
                "当前微信已绑定其他来源；没有合并或改绑旧资料",
            ));
        }
        tx.execute("INSERT INTO setup_source_accounts(source_fingerprint,account_id) VALUES(?1,?2) ON CONFLICT(source_fingerprint) DO NOTHING",params![fingerprint,account]).map_err(|_|AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn setup_account_for_source_store(
        &self,
        label: &str,
        fingerprint: &str,
    ) -> AppResult<Account> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("INVALID_ARGUMENT", "来源指纹无效"));
        }
        let fingerprint = fingerprint.to_ascii_lowercase();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let account = Self::claim_current(&tx, label, true)?;
        let mapped: Option<String> = tx
            .query_row(
                "SELECT source_fingerprint FROM setup_source_accounts WHERE account_id=?1",
                [&account.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let owner: Option<String> = tx
            .query_row(
                "SELECT account_id FROM setup_source_accounts WHERE source_fingerprint=?1",
                [&fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        if mapped.as_ref().is_some_and(|v| v != &fingerprint)
            || owner.as_ref().is_some_and(|id| id != &account.id)
        {
            return Err(AppError::new(
                "SOURCE_CONFLICT",
                "来源与当前微信不一致；没有合并旧资料或改绑密钥",
            ));
        }
        tx.execute("INSERT INTO setup_source_accounts(source_fingerprint,account_id) VALUES(?1,?2) ON CONFLICT(source_fingerprint) DO NOTHING", params![fingerprint,account.id]).map_err(|_| AppError::internal())?;
        tx.execute(
            "UPDATE setup_state SET account_id=?1 WHERE singleton=1",
            [&account.id],
        )
        .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(account)
    }

    pub(crate) fn setup_account_store(&self, label: &str) -> AppResult<Account> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let account = Self::claim_current(&tx, label, true)?;
        tx.execute(
            "UPDATE setup_state SET account_id=?1 WHERE singleton=1",
            [&account.id],
        )
        .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(account)
    }
}
