use crate::*;
use rusqlite::{Transaction, TransactionBehavior};
use werc_domain::current::CurrentConnection;

pub(crate) fn require_current(tx: &Connection, account: &str) -> AppResult<()> {
    let current: Option<String> = tx
        .query_row(
            "SELECT account_id FROM current_connection WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| AppError::internal())?;
    if current.as_deref() != Some(account) {
        return Err(AppError::new(
            "CURRENT_CONNECTION_REQUIRED",
            "此资料仅供归档浏览，不能连接或继续来源操作",
        ));
    }
    Ok(())
}

impl SqliteRepository {
    pub(crate) fn current_connection_store(&self) -> AppResult<CurrentConnection> {
        self.connection()?.query_row(
            "SELECT account_id,revision,selection_required FROM current_connection WHERE singleton=1", [],
            |r| Ok(CurrentConnection { account_id:r.get(0)?, revision:r.get::<_, i64>(1)?.to_string(), selection_required:r.get(2)?, identity_verified:false })
        ).map_err(|_| AppError::internal())
    }

    pub(crate) fn select_current_store(&self, account: &str, revision: &str) -> AppResult<()> {
        let expected = revision
            .parse::<i64>()
            .map_err(|_| AppError::new("INVALID_ARGUMENT", "连接修订号无效"))?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (selected, current, required): (Option<String>, i64, bool) = tx.query_row(
            "SELECT account_id,revision,selection_required FROM current_connection WHERE singleton=1", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))
        ).map_err(|_| AppError::internal())?;
        if current != expected {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "当前连接状态已变化，请重新查看",
            ));
        }
        if selected.is_some() || !required {
            return Err(AppError::new(
                "SINGLE_CONNECTION_REQUIRED",
                "当前微信已确定；不提供多微信切换",
            ));
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",
                [account],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if !exists {
            return Err(AppError::new("ACCOUNT_NOT_FOUND", "旧资料不存在"));
        }
        tx.execute("UPDATE current_connection SET account_id=?1,revision=revision+1,selection_required=0 WHERE singleton=1", [account]).map_err(|_| AppError::internal())?;
        // 旧令牌不因本次选择自动恢复或改绑。
        tx.execute("UPDATE tokens SET revoked=1", [])
            .map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'connection.selected',?2,'{}')", params![account,Utc::now().to_rfc3339()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }

    pub(crate) fn claim_current(
        tx: &Transaction<'_>,
        label: &str,
        reuse: bool,
    ) -> AppResult<Account> {
        let (id, required): (Option<String>, bool) = tx
            .query_row(
                "SELECT account_id,selection_required FROM current_connection WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| AppError::internal())?;
        if required {
            return Err(AppError::new(
                "MIGRATION_SELECTION_REQUIRED",
                "请先确认唯一当前来源；旧资料保持不变",
            ));
        }
        if let Some(id) = id {
            if !reuse {
                return Err(AppError::new(
                    "SINGLE_CONNECTION_REQUIRED",
                    "此软件只连接一个本机微信，不能重复登记",
                ));
            }
            return tx
                .query_row(
                    "SELECT id,label,created_at FROM accounts WHERE id=?1",
                    [id],
                    |r| {
                        Ok(Account {
                            id: r.get(0)?,
                            label: r.get(1)?,
                            created_at: r.get(2)?,
                            state: AccountState::Registered,
                        })
                    },
                )
                .map_err(|_| AppError::internal());
        }
        let account = Account {
            id: Uuid::new_v4().to_string(),
            label: label.into(),
            created_at: Utc::now().to_rfc3339(),
            state: AccountState::Registered,
        };
        tx.execute(
            "INSERT INTO accounts(id,label,created_at) VALUES(?1,?2,?3)",
            params![account.id, account.label, account.created_at],
        )
        .map_err(|_| AppError::internal())?;
        tx.execute(
            "UPDATE current_connection SET account_id=?1,revision=revision+1 WHERE singleton=1",
            [&account.id],
        )
        .map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'account.registered',?2,'{}')", params![account.id,account.created_at]).map_err(|_| AppError::internal())?;
        Ok(account)
    }
}
