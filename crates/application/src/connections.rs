use crate::Application;
use chrono::Utc;
use werc_domain::{
    AccountState, AppError, AppResult, Principal,
    connection::{ClientBinding, ClientInstallation, DiscoveryReport, KeyStatus},
};
use zeroize::Zeroizing;

#[derive(serde::Serialize)]
pub struct RevealedDatabaseKey {pub salt:Option<String>,pub key_hex:String,pub verified:bool}
impl Drop for RevealedDatabaseKey {fn drop(&mut self){use zeroize::Zeroize;self.key_hex.zeroize();}}

impl Application {
    /// Explicit local UI action only. Never included in bootstrap, logs or HTTP responses.
    pub fn reveal_database_keys_local(&self,account:&str)->AppResult<Vec<RevealedDatabaseKey>>{
        self.ensure_account(account)?;
        let bytes=self.live_keys_local(account)?;
        let count=crate::keyring::verified_count(&bytes)?;
        if count==0{return Ok(vec![RevealedDatabaseKey{salt:None,key_hex:hex::encode(&*bytes),verified:false}]);}
        Ok(bytes[8..].chunks_exact(80).map(|r|RevealedDatabaseKey{salt:Some(hex::encode(&r[..16])),key_hex:hex::encode(&r[16..48]),verified:true}).collect())
    }
    pub fn discover_clients_local(&self) -> AppResult<DiscoveryReport> {
        self.provider.discover()
    }
    pub fn inspect_client_local(&self, path: &str) -> AppResult<ClientInstallation> {
        self.provider.inspect(path)
    }
    pub(crate) fn ensure_account(&self, account: &str) -> AppResult<()> {
        if self.repository.accounts()?.iter().any(|a| a.id == account) {
            Ok(())
        } else {
            Err(AppError::new("ACCOUNT_NOT_FOUND", "登记账号不存在"))
        }
    }
    pub fn bindings_local(&self) -> AppResult<Vec<ClientBinding>> {
        self.repository.bindings()
    }
    pub fn binding(
        &self,
        principal: &Principal,
        account: &str,
    ) -> AppResult<Option<ClientBinding>> {
        principal.require("connections:read")?;
        principal.require_account(account)?;
        self.ensure_account(account)?;
        Ok(self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account))
    }
    pub fn bind_client_local(&self, account: &str, path: &str) -> AppResult<ClientBinding> {
        self.ensure_account(account)?;
        self.ensure_current_connection(account)?;
        let installation = self.provider.inspect(path)?;
        let now = Utc::now().to_rfc3339();
        let previous = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account);
        let unchanged = previous.as_ref().is_some_and(|b| {
            b.installation.id == installation.id && b.installation.sha256 == installation.sha256
        });
        let binding = ClientBinding {
            account_id: account.into(),
            installation,
            checked_at: now.clone(),
            bound_at: if unchanged {
                previous.map(|b| b.bound_at).unwrap_or_else(|| now.clone())
            } else {
                now
            },
            validation: "metadata_only".into(),
        };
        self.repository.save_binding(&binding)?;
        Ok(binding)
    }
    pub fn unbind_client_local(&self, account: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.remove_binding(account)
    }
    pub fn import_key_local(&self, account: &str, key_hex: String) -> AppResult<KeyStatus> {
        let text = Zeroizing::new(key_hex);
        self.ensure_account(account)?;
        let binding = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account)
            .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "请先选择并绑定客户端来源"))?;
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "数据库密钥需要 64 位十六进制字符；未保存输入",
            ));
        }
        let key = Zeroizing::new(
            hex::decode(text.as_str())
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "密钥格式不正确"))?,
        );
        let context = format!("WeRC/key/v1/{account}/{}", binding.installation.sha256);
        let protected = self.provider.protect_key(&context, &key)?;
        let restored = Zeroizing::new(self.provider.unprotect_key(&context, &protected)?);
        if restored.as_slice() != key.as_slice() {
            return Err(AppError::new(
                "KEY_PROTECTION_FAILED",
                "密钥保护回读校验未通过",
            ));
        }
        self.repository.store_key(&binding, &protected)?;
        self.key_status_local(account)
    }
    pub fn acquire_database_key_local(
        &self,
        account: &str,
        process_id: u32,
        database: &str,
        confirmed: bool,
    ) -> AppResult<KeyStatus> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "取钥需要确认只读指定进程和所选数据库第一页",
            ));
        }
        self.ensure_account(account)?;
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        self.acquire_database_key_guarded(account, process_id, database)
    }
    /// Caller holds history_guard so foreground reads and key updates cannot overlap.
    pub(crate) fn acquire_database_key_guarded(
        &self,
        account: &str,
        process_id: u32,
        database: &str,
    ) -> AppResult<KeyStatus> {
        self.acquire_database_key_inner(account, process_id, database, None)
    }
    pub fn acquire_database_key_cancellable_local(
        &self, account: &str, process_id: u32, database: &str, confirmed: bool,
        request: &str, timeout_seconds: u64, progress: &(dyn Fn(u64) + Send + Sync),
    ) -> AppResult<KeyStatus> {
        if !confirmed { return Err(AppError::new("CONSENT_REQUIRED", "取钥需要确认只读指定进程和数据库第一页")); }
        if !(1..=30).contains(&timeout_seconds) { return Err(AppError::new("INVALID_ARGUMENT", "扫描时间应为1—30秒")); }
        self.ensure_account(account)?;
        let _guard = self.history_guard.try_lock().map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let lease = self.browse_lease(request)?;
        let control = crate::ports::KeyScanControl { cancel: &lease.cancel, deadline: std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds), progress };
        control.check()?;
        self.acquire_database_key_inner(account, process_id, database, Some(&control))
    }
    fn acquire_database_key_inner(
        &self, account: &str, process_id: u32, database: &str,
        control: Option<&crate::ports::KeyScanControl<'_>>,
    ) -> AppResult<KeyStatus> {
        let binding = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account)
            .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "请先绑定指定客户端"))?;
        let revision = self.repository.key_revision(account)?;
        let context = format!("WeRC/key/v1/{account}/{}", binding.installation.sha256);
        let previous = self
            .repository
            .protected_key(account)?
            .map(|(bytes, _)| {
                self.provider
                    .unprotect_key(&context, &bytes)
                    .map(Zeroizing::new)
            })
            .transpose()?;
        let acquired = match control {
            Some(control) => self.provider.acquire_database_key_controlled(&binding.installation, process_id, database, control)?,
            None => self.provider.acquire_database_key(&binding.installation, process_id, database)?,
        };
        let payload = crate::keyring::merge(previous.as_deref().map(Vec::as_slice), &acquired)?;
        let protected = self.provider.protect_key(&context, &payload)?;
        let restored = Zeroizing::new(self.provider.unprotect_key(&context, &protected)?);
        if restored.as_slice() != payload.as_slice() {
            return Err(AppError::new(
                "KEY_PROTECTION_FAILED",
                "密钥集保护回读不一致",
            ));
        }
        if let Some(control) = control { control.check()?; }
        self.repository
            .store_acquired_key(&binding, &protected, revision)?;
        self.key_status_local(account)
    }
    pub fn key_status_local(&self, account: &str) -> AppResult<KeyStatus> {
        self.ensure_account(account)?;
        let stored = self.repository.protected_key(account)?;
        let mut verified_databases = 0;
        let mut verification_error = None;
        if let Some((protected, _)) = &stored {
            let result = (|| {
                let binding = self
                    .repository
                    .bindings()?
                    .into_iter()
                    .find(|b| b.account_id == account)
                    .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "密钥绑定不存在"))?;
                let bytes = Zeroizing::new(self.provider.unprotect_key(
                    &format!("WeRC/key/v1/{account}/{}", binding.installation.sha256),
                    protected,
                )?);
                crate::keyring::verified_count(&bytes)
            })();
            match result {
                Ok(count) => verified_databases = count,
                Err(error) => verification_error = Some(error.code),
            }
        }
        Ok(KeyStatus {
            account_id: account.into(),
            protected: stored.is_some(),
            verified: verified_databases > 0,
            verified_databases,
            verification_error,
            updated_at: stored.map(|(_, date)| date),
        })
    }
    pub fn remove_key_local(&self, account: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.remove_key(account)
    }
    pub(crate) fn account_state(&self, account: &str) -> AppResult<AccountState> {
        let binding = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account);
        Ok(match binding {
            None => AccountState::Registered,
            Some(binding) if binding.installation.compatibility == "unsupported" => {
                AccountState::Unsupported
            }
            Some(_) => AccountState::NeedsKey,
        })
    }
}
