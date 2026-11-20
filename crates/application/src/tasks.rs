use crate::Application;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use werc_domain::{
    AppError, AppResult, Principal,
    media::inspect_wav,
    operation::{Operation, OperationState},
};

impl Application {
    pub fn inspect_audio_task(
        &self,
        principal: &Principal,
        account: &str,
        asset: &str,
        key: &str,
    ) -> AppResult<Operation> {
        principal.require("operations:create")?;
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.queue_audio_task(&principal.id, account, asset, key)
    }
    pub fn inspect_audio_task_local(
        &self,
        account: &str,
        asset: &str,
        key: &str,
    ) -> AppResult<Operation> {
        self.queue_audio_task("desktop", account, asset, key)
    }
    fn queue_audio_task(
        &self,
        creator: &str,
        account: &str,
        asset: &str,
        key: &str,
    ) -> AppResult<Operation> {
        self.ensure_account(account)?;
        if !(16..=128).contains(&key.len())
            || !key.bytes().all(|b| b.is_ascii_graphic())
            || Uuid::parse_str(asset).is_err()
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "需要有效资产与 16–128 字符幂等键",
            ));
        }
        let now = Utc::now().to_rfc3339();
        let digest = hex::encode(Sha256::digest(format!("audio.inspect\n{account}\n{asset}")));
        self.repository.queue_operation(
            &Operation {
                id: Uuid::new_v4().to_string(),
                account_id: account.into(),
                creator_id: creator.into(),
                kind: "audio.inspect".into(),
                asset_id: asset.into(),
                request_digest: digest,
                state: OperationState::Queued,
                created_at: now.clone(),
                updated_at: now,
                result: None,
                error_code: None,
            },
            key,
        )
    }
    pub fn operations(&self, principal: &Principal, account: &str) -> AppResult<Vec<Operation>> {
        principal.require("operations:read")?;
        principal.require_account(account)?;
        self.operations_local(account)
    }
    pub fn operations_local(&self, account: &str) -> AppResult<Vec<Operation>> {
        self.ensure_account(account)?;
        self.repository.operations(account)
    }
    pub fn operation(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<Operation> {
        principal.require("operations:read")?;
        principal.require_account(account)?;
        self.repository.operation(account, id)
    }
    pub fn cancel_operation(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<()> {
        principal.require("operations:create")?;
        principal.require_account(account)?;
        self.cancel_operation_local(account, id)
    }
    pub fn cancel_operation_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.repository.cancel_operation(account, id)
    }
    pub fn recover_local_tasks(&self) -> AppResult<usize> {
        self.repository.recover_operations()
    }
    pub fn run_local_task_once(&self) -> AppResult<bool> {
        let Some(mut operation) = self.repository.claim_operation()? else {
            return Ok(false);
        };
        let result = (|| {
            if operation.kind != "audio.inspect" {
                return Err(AppError::new(
                    "CAPABILITY_NOT_IMPLEMENTED",
                    "任务类型没有本地执行器",
                ));
            }
            let bytes = self.decrypt_audio_local(&operation.account_id, &operation.asset_id)?;
            let info = inspect_wav(&bytes)?;
            Ok(serde_json::json!({"audio":info,"sent":false,"execution":"local_only"}))
        })();
        operation.updated_at = Utc::now().to_rfc3339();
        match result {
            Ok(value) => {
                operation.state = operation
                    .state
                    .finish_local_analysis(OperationState::Succeeded)?;
                operation.result = Some(value);
            }
            Err(error) => {
                operation.state = operation
                    .state
                    .finish_local_analysis(OperationState::Failed)?;
                operation.error_code = Some(error.code);
            }
        }
        self.repository.finish_operation(&operation)?;
        Ok(true)
    }
}
