use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, Transaction, params};
use werc_domain::{
    AppError, AppResult,
    operation::{Operation, OperationState},
};

fn decode(json: &str) -> AppResult<Operation> {
    serde_json::from_str(json).map_err(|_| AppError::internal())
}
fn state_name(state: OperationState) -> AppResult<String> {
    serde_json::to_value(state)
        .map_err(|_| AppError::internal())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(AppError::internal)
}
fn save(tx: &Transaction<'_>, operation: &Operation) -> AppResult<()> {
    tx.execute(
        "UPDATE operations SET state=?1,record=?2 WHERE id=?3",
        params![
            state_name(operation.state)?,
            serde_json::to_string(operation).map_err(|_| AppError::internal())?,
            operation.id
        ],
    )
    .map_err(|_| AppError::internal())?;
    event(tx, operation)
}
fn event(tx: &Transaction<'_>, operation: &Operation) -> AppResult<()> {
    tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'operation.updated',?2,?3)", params![operation.account_id, operation.updated_at, serde_json::json!({"operation_id":operation.id,"kind":operation.kind,"state":operation.state,"sent":false}).to_string()]).map_err(|_| AppError::internal())?;
    Ok(())
}
impl SqliteRepository {
    pub(crate) fn queue_operation_store(
        &self,
        operation: &Operation,
        key: &str,
    ) -> AppResult<Operation> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let existing: Option<String> = tx.query_row("SELECT record FROM operations WHERE account_id=?1 AND creator_id=?2 AND idempotency_key=?3", params![operation.account_id, operation.creator_id, key], |r| r.get(0)).optional().map_err(|_| AppError::internal())?;
        if let Some(json) = existing {
            let previous = decode(&json)?;
            if previous.request_digest != operation.request_digest {
                return Err(AppError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "此幂等键已用于不同的请求",
                ));
            }
            return Ok(previous);
        }
        let asset_exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM audio_assets WHERE id=?1 AND account_id=?2 AND expires_at>?3)", params![operation.asset_id, operation.account_id, Utc::now().to_rfc3339()], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if !asset_exists {
            return Err(AppError::new("ASSET_NOT_FOUND", "音频不存在或已过期"));
        }
        let pending: i64 = tx.query_row("SELECT count(*) FROM operations WHERE account_id=?1 AND state IN ('queued','preparing')", [&operation.account_id], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if pending >= 128 {
            return Err(AppError::new("BUSY", "账号本地任务队列已满"));
        }
        tx.execute("INSERT INTO operations(id,account_id,creator_id,idempotency_key,state,record) VALUES (?1,?2,?3,?4,'queued',?5)", params![operation.id, operation.account_id, operation.creator_id, key, serde_json::to_string(operation).map_err(|_| AppError::internal())?]).map_err(|_| AppError::internal())?;
        event(&tx, operation)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(operation.clone())
    }
    pub(crate) fn operations_store(&self, account: &str) -> AppResult<Vec<Operation>> {
        let c = self.connection()?;
        let mut s = c
            .prepare(
                "SELECT record FROM operations WHERE account_id=?1 ORDER BY rowid DESC LIMIT 200",
            )
            .map_err(|_| AppError::internal())?;
        s.query_map([account], |r| r.get::<_, String>(0))
            .map_err(|_| AppError::internal())?
            .map(|r| decode(&r.map_err(|_| AppError::internal())?))
            .collect()
    }
    pub(crate) fn operation_store(&self, account: &str, id: &str) -> AppResult<Operation> {
        let c = self.connection()?;
        let json: String = c
            .query_row(
                "SELECT record FROM operations WHERE account_id=?1 AND id=?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?
            .ok_or_else(|| AppError::new("OPERATION_NOT_FOUND", "任务不存在"))?;
        decode(&json)
    }
    pub(crate) fn claim_operation_store(&self) -> AppResult<Option<Operation>> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let rows: Vec<String> = {
            let mut s = tx
                .prepare(
                    "SELECT record FROM operations WHERE state='queued' ORDER BY rowid LIMIT 129",
                )
                .map_err(|_| AppError::internal())?;
            s.query_map([], |r| r.get(0))
                .map_err(|_| AppError::internal())?
                .collect::<Result<_, _>>()
                .map_err(|_| AppError::internal())?
        };
        for json in rows {
            let mut op = decode(&json)?;
            let authorized = op.creator_id == "desktop" || tx.query_row("SELECT EXISTS(SELECT 1 FROM tokens JOIN token_accounts ON tokens.id=token_accounts.token_id WHERE tokens.id=?1 AND tokens.revoked=0 AND (tokens.expires_at>?2 OR tokens.expires_at=?4) AND token_accounts.account_id=?3)", params![op.creator_id, Utc::now().to_rfc3339(), op.account_id,werc_domain::PERMANENT_TOKEN_EXPIRY], |r| r.get::<_, bool>(0)).map_err(|_| AppError::internal())?;
            op.updated_at = Utc::now().to_rfc3339();
            if !authorized {
                op.state = op.state.transition(OperationState::Canceled)?;
                op.error_code = Some("AUTH_REVOKED".into());
                save(&tx, &op)?;
                continue;
            }
            op.state = op.state.transition(OperationState::Preparing)?;
            save(&tx, &op)?;
            tx.commit().map_err(|_| AppError::internal())?;
            return Ok(Some(op));
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(None)
    }
    pub(crate) fn finish_operation_store(&self, operation: &Operation) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let current: Option<String> = tx
            .query_row(
                "SELECT record FROM operations WHERE id=?1 AND account_id=?2",
                params![operation.id, operation.account_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        if let Some(json) = current {
            let previous = decode(&json)?;
            if previous.state == OperationState::Canceled {
                return Ok(());
            }
            if previous.state != OperationState::Preparing
                || !matches!(
                    operation.state,
                    OperationState::Succeeded | OperationState::Failed
                )
            {
                return Err(AppError::new(
                    "INVALID_TRANSITION",
                    "本地任务已被其他操作修改",
                ));
            }
            let authorized = previous.creator_id == "desktop" || tx.query_row("SELECT EXISTS(SELECT 1 FROM tokens JOIN token_accounts ON tokens.id=token_accounts.token_id WHERE tokens.id=?1 AND tokens.revoked=0 AND (tokens.expires_at>?2 OR tokens.expires_at=?4) AND token_accounts.account_id=?3)", params![previous.creator_id,Utc::now().to_rfc3339(),previous.account_id,werc_domain::PERMANENT_TOKEN_EXPIRY],|r|r.get::<_,bool>(0)).map_err(|_|AppError::internal())?;
            if !authorized {
                let mut canceled = previous.clone();
                canceled.state = previous.state.transition(OperationState::Canceled)?;
                canceled.updated_at = Utc::now().to_rfc3339();
                canceled.error_code = Some("AUTH_REVOKED".into());
                canceled.result = None;
                save(&tx, &canceled)?;
            } else {
                save(&tx, operation)?;
            }
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn cancel_operation_store(&self, account: &str, id: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let json: String = tx
            .query_row(
                "SELECT record FROM operations WHERE id=?1 AND account_id=?2",
                params![id, account],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?
            .ok_or_else(|| AppError::new("OPERATION_NOT_FOUND", "任务不存在"))?;
        let mut op = decode(&json)?;
        op.state = op.state.transition(OperationState::Canceled)?;
        op.updated_at = Utc::now().to_rfc3339();
        save(&tx, &op)?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn recover_operations_store(&self) -> AppResult<usize> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let rows: Vec<String> = {
            let mut s = tx
                .prepare("SELECT record FROM operations WHERE state IN ('preparing','submitting')")
                .map_err(|_| AppError::internal())?;
            s.query_map([], |r| r.get(0))
                .map_err(|_| AppError::internal())?
                .collect::<Result<_, _>>()
                .map_err(|_| AppError::internal())?
        };
        let count = rows.len();
        for json in rows {
            let mut op = decode(&json)?;
            op.state = if op.state == OperationState::Submitting {
                OperationState::Unknown
            } else {
                OperationState::Failed
            };
            op.error_code = Some("INTERRUPTED".into());
            op.updated_at = Utc::now().to_rfc3339();
            save(&tx, &op)?;
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(count)
    }
}
