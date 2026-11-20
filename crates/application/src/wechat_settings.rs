use crate::Application;
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, Principal, wechat_settings::*};

impl Application {
    fn settings_access(&self, p: &Principal, account: &str, write: bool) -> AppResult<()> {
        p.require(if write {
            "wechat:settings:write"
        } else {
            "wechat:settings:read"
        })?;
        p.require_account(account)?;
        self.ensure_account(account)?;
        self.ensure_current_connection(account)
    }
    pub fn wechat_settings_capabilities(
        &self,
        p: &Principal,
        account: &str,
    ) -> AppResult<SettingsCapabilities> {
        self.settings_access(p, account, false)?;
        match self.provider.wechat_settings_read(account) {
            Ok(s) => Ok(SettingsCapabilities {
                available: true,
                build: Some(s.build),
                items: s
                    .values
                    .iter()
                    .map(|v| SettingDescriptor::native(v.key))
                    .collect(),
                reason: None,
            }),
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "WECHAT_SETTINGS_UNAVAILABLE"
                        | "NATIVE_NOT_CONNECTED"
                        | "NATIVE_DISCONNECTED"
                        | "NATIVE_CONTEXT_CHANGED"
                        | "NATIVE_ADAPTER_PENDING"
                        | "NATIVE_BUILD_UNSUPPORTED"
                ) =>
            {
                Ok(SettingsCapabilities {
                    available: false,
                    build: None,
                    items: vec![],
                    reason: Some(e.code),
                })
            }
            Err(e) => Err(e),
        }
    }
    pub fn wechat_settings_read(
        &self,
        p: &Principal,
        account: &str,
    ) -> AppResult<SettingsSnapshot> {
        self.settings_access(p, account, false)?;
        self.provider.wechat_settings_read(account)
    }
    pub fn wechat_settings_plan(
        &self,
        p: &Principal,
        account: &str,
        target: &SettingTarget,
    ) -> AppResult<SettingPlan> {
        self.settings_access(p, account, false)?;
        let snapshot = self.provider.wechat_settings_read(account)?;
        let before = setting(&snapshot, target.key)?;
        Ok(SettingPlan {
            key: target.key,
            before,
            requested: target.value,
            action: if target.expected_value.is_some_and(|e| e != before) {
                "conflict"
            } else if before == target.value {
                "unchanged"
            } else {
                "set"
            }
            .into(),
            native_epoch: snapshot.native_epoch,
            build: snapshot.build,
            requires_foreground: false,
        })
    }
    pub fn wechat_settings_apply(
        &self,
        p: &Principal,
        account: &str,
        target: &SettingTarget,
        key: &str,
    ) -> AppResult<SettingsChange> {
        self.settings_apply_inner(p, account, target, key, "")
    }
    fn settings_apply_inner(
        &self,
        p: &Principal,
        account: &str,
        target: &SettingTarget,
        key: &str,
        restore_id: &str,
    ) -> AppResult<SettingsChange> {
        self.settings_access(p, account, true)?;
        if !(16..=128).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "idempotency_key 需要 16–128 个可见 ASCII 字符",
            ));
        }
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        let request = hex::encode(Sha256::digest(
            serde_json::to_vec(&(target, restore_id)).map_err(|_| AppError::internal())?,
        ));
        // Only settings writes serialize. Independent message/database reads keep their own locks.
        let _guard = self
            .settings_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        self.settings_access(p, account, true)?;
        if let Some(mut existing) = self
            .repository
            .settings_change_by_key(account, &p.id, &digest)?
        {
            if existing.request_digest != request {
                return Err(AppError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "此幂等键已用于其他设置目标",
                ));
            }
            existing.replayed = true;
            return Ok(existing);
        }
        let snapshot = self.provider.wechat_settings_read(account)?;
        let before = setting(&snapshot, target.key)?;
        if !restore_id.is_empty() {
            let old = self
                .repository
                .settings_change(account, restore_id)?
                .ok_or_else(|| AppError::new("NOT_FOUND", "设置记录不存在"))?;
            if old.native_epoch != snapshot.native_epoch || old.build != snapshot.build {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "原设置记录所属微信连接已变化，请读取当前设置后重新指定目标",
                ));
            }
        }
        let mut change = SettingsChange {
            id: uuid::Uuid::new_v4().to_string(),
            account_id: account.into(),
            creator_id: p.id.clone(),
            idempotency_digest: digest,
            request_digest: request,
            created_at: chrono::Utc::now().to_rfc3339(),
            target: target.clone(),
            native_epoch: snapshot.native_epoch.clone(),
            build: snapshot.build,
            result: SettingResult {
                status: ChangeStatus::Unknown,
                before: Some(before),
                requested: target.value,
                observed: None,
                error_code: Some("SETTINGS_RESULT_PENDING".into()),
            },
            replayed: false,
        };
        // Persist before invoking native code: a crash or timeout must never cause a retry to submit again.
        self.repository.settings_change_save(&change)?;
        change.result =
            match self
                .provider
                .wechat_settings_apply(account, target, &change.native_epoch)
            {
                Ok(r) => r,
                Err(e) => SettingResult {
                    status: ChangeStatus::Unknown,
                    before: Some(before),
                    requested: target.value,
                    observed: None,
                    error_code: Some(e.code),
                },
            };
        self.repository.settings_change_save(&change)?;
        Ok(change)
    }
    pub fn wechat_settings_changes(
        &self,
        p: &Principal,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<Vec<(i64, SettingsChange)>> {
        self.settings_access(p, account, false)?;
        if before.is_some_and(|n| n <= 0) || !(1..=100).contains(&limit) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "limit 需为 1–100，before 需为正整数",
            ));
        }
        self.repository.settings_changes(account, before, limit)
    }
    pub fn wechat_settings_restore(
        &self,
        p: &Principal,
        account: &str,
        id: &str,
        key: &str,
    ) -> AppResult<SettingsChange> {
        self.settings_access(p, account, true)?;
        let old = self
            .repository
            .settings_change(account, id)?
            .ok_or_else(|| AppError::new("NOT_FOUND", "设置记录不存在"))?;
        if !matches!(
            old.result.status,
            ChangeStatus::Verified | ChangeStatus::Unchanged
        ) {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "仅已验证且旧值已知的设置记录可恢复；其他结果请先读取当前设置",
            ));
        }
        let before = old
            .result
            .before
            .ok_or_else(|| AppError::new("INVALID_TRANSITION", "原值未知"))?;
        self.settings_apply_inner(
            p,
            account,
            &SettingTarget {
                key: old.target.key,
                value: before,
                expected_value: old.result.observed,
            },
            key,
            id,
        )
    }
}
fn setting(s: &SettingsSnapshot, key: SettingKey) -> AppResult<bool> {
    s.values
        .iter()
        .find(|v| v.key == key)
        .map(|v| v.value)
        .ok_or_else(|| {
            AppError::new(
                "WECHAT_SETTINGS_UNAVAILABLE",
                "当前精确构建未开放此后台原生设置",
            )
        })
}
