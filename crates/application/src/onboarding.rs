use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::atomic::Ordering};
use uuid::Uuid;
use werc_domain::{AppError, AppResult, history::HistoryReadRequest, onboarding::*};
use zeroize::Zeroizing;

const CONTEXT: &str = "WeRC/onboarding/v1";
const MAX_SETUP_BYTES: usize = 8 * 1024 * 1024;
const MAX_DATABASES: usize = 512;
const READER_MAX_BYTES: u64 = 256 * 1024 * 1024;

// This is a local isolation scope, NOT proof of a WeChat account identity.
// Never choose an arbitrary common ancestor: it could merge two account roots.
// Known db_storage layouts may have nested category directories; other layouts
// must share the immediate database parent directory.
pub(crate) fn source_root(path: &Path) -> AppResult<String> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "数据库没有有效来源目录"))?;
    let root = parent
        .ancestors()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("db_storage"))
        })
        .and_then(Path::parent)
        .unwrap_or(parent);
    if root.parent().is_none() {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "不能以磁盘根目录作为来源范围",
        ));
    }
    Ok(root.to_string_lossy().to_lowercase())
}

fn settle_error(run: &mut SetupRun, index: usize, error: AppError) -> bool {
    if matches!(
        error.code.as_str(),
        "BUSY" | "SOURCE_BUSY" | "SOURCE_BUSY_OR_DENIED" | "SETUP_PAUSED"
    ) {
        run.phase = "waiting".into();
        return false;
    }
    let item = &mut run.items[index];
    item.error = Some(error.code.clone());
    match error.code.as_str() {
        "KEY_NOT_FOUND" | "KEY_SCAN_INCOMPLETE" => {
            // No verified candidate, including a bounded scan that could not
            // finish, is an availability gap, not proof that other keys are unsafe.
            // Preserve the error and report a partial result, never a success.
            item.skipped = true;
            item.outcome = Some("key_deferred".into());
            false
        }
        "SCHEMA_UNSUPPORTED" | "SCHEMA_EXTENSION_UNSUPPORTED" => {
            item.skipped = true;
            item.outcome = Some("unsupported".into());
            false
        }
        "SOURCE_LIMIT_EXCEEDED"
        | "DATABASE_LIMIT_EXCEEDED"
        | "MEMORY_LIMIT_EXCEEDED"
        | "CONTENT_LIMIT_EXCEEDED"
        | "RESULT_LIMIT_EXCEEDED"
        | "SCHEMA_LIMIT_EXCEEDED"
        | "ROW_LIMIT_EXCEEDED"
        | "QUOTA_EXCEEDED" => {
            item.skipped = true;
            item.outcome = Some("resource_deferred".into());
            false
        }
        _ => {
            // Fail closed for path, binding, component and page authentication
            // failures (and unknown errors); resume needs renewed confirmation.
            item.outcome = Some("failed".into());
            run.phase = "failed".into();
            true
        }
    }
}
fn busy() -> AppError {
    AppError::new("BUSY", "引导当前步骤尚未结束")
}
fn consent(value: bool) -> AppResult<()> {
    if value {
        Ok(())
    } else {
        Err(AppError::new(
            "CONSENT_REQUIRED",
            "请确认所选来源的只读取钥与离线导入",
        ))
    }
}
impl Application {
    fn load_setup(&self) -> AppResult<Option<SetupRun>> {
        let Some(payload) = self.repository.setup_payload()? else {
            return Ok(None);
        };
        if payload.len() > 32 * 1024 * 1024 {
            return Err(AppError::new("SETUP_CORRUPTED", "引导状态超限"));
        }
        let envelope: ProtectedSetup =
            serde_json::from_slice(&payload).map_err(|_| AppError::internal())?;
        if envelope.nonce.len() != 12 {
            return Err(AppError::new(
                "SETUP_CORRUPTED",
                "引导状态损坏，可重置引导；不影响已保存记录",
            ));
        }
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(CONTEXT, &envelope.wrapped_key)?,
        );
        let bytes = Zeroizing::new(
            Aes256Gcm::new_from_slice(&key)
                .map_err(|_| AppError::internal())?
                .decrypt(
                    Nonce::from_slice(&envelope.nonce),
                    Payload {
                        msg: &envelope.ciphertext,
                        aad: CONTEXT.as_bytes(),
                    },
                )
                .map_err(|_| AppError::new("SETUP_CORRUPTED", "引导状态完整性校验失败"))?,
        );
        if bytes.len() > MAX_SETUP_BYTES {
            return Err(AppError::new("SETUP_CORRUPTED", "引导状态超限"));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| AppError::internal())
    }
    fn save_setup(&self, run: &mut SetupRun) -> AppResult<()> {
        run.revision += 1;
        let bytes = Zeroizing::new(serde_json::to_vec(run).map_err(|_| AppError::internal())?);
        if bytes.len() > MAX_SETUP_BYTES {
            return Err(AppError::new("QUOTA_EXCEEDED", "引导来源范围过大"));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let wrapped_key = self.provider.protect_key(CONTEXT, key.as_ref())?;
        let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| AppError::internal())?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &bytes,
                    aad: CONTEXT.as_bytes(),
                },
            )
            .map_err(|_| AppError::internal())?;
        let payload = serde_json::to_vec(&ProtectedSetup {
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
        .map_err(|_| AppError::internal())?;
        self.repository.save_setup_payload(Some(&payload))
    }
    pub fn setup_status_local(&self) -> AppResult<SetupStatus> {
        Ok(SetupStatus {
            run: self.load_setup()?,
            active: self.setup_active.load(Ordering::SeqCst),
            working: self.setup_guard.try_lock().is_err(),
            dismissed: self.repository.setup_dismissed()?,
        })
    }
    pub fn setup_pause_local(&self) -> AppResult<()> {
        self.setup_active.store(false, Ordering::SeqCst);
        Ok(())
    }
    pub fn setup_dismiss_local(&self) -> AppResult<()> {
        self.setup_pause_local()?;
        self.repository.dismiss_setup()
    }
    pub fn setup_reset_local(&self) -> AppResult<()> {
        self.setup_pause_local()?;
        let _guard = self.setup_guard.try_lock().map_err(|_| busy())?;
        self.repository.save_setup_payload(None)
    }
    fn check_setup_binding(&self, run: &SetupRun) -> AppResult<()> {
        self.ensure_current_connection(&run.binding.account_id)?;
        let binding = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == run.binding.account_id);
        if binding.as_ref().is_none_or(|b| {
            b.installation.sha256 != run.binding.installation.sha256
                || b.installation.id != run.binding.installation.id
                || b.bound_at != run.binding.bound_at
        }) || self.repository.key_revision(&run.binding.account_id)? != run.key_revision
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "绑定或密钥已变化，请重置并重新确认引导范围",
            ));
        }
        let current = self
            .provider
            .inspect(&run.binding.installation.executable_path)?;
        if current.sha256 != run.binding.installation.sha256 {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "安装文件已变化，请重新绑定",
            ));
        }
        Ok(())
    }
    pub fn setup_start_local(
        &self,
        request: SetupRequest,
        confirmed: bool,
    ) -> AppResult<SetupStatus> {
        consent(confirmed)?;
        {
            let _guard = self.setup_guard.try_lock().map_err(|_| busy())?;
            if self.load_setup()?.is_some() {
                return Err(AppError::new("SETUP_EXISTS", "已有引导，请继续或明确重置"));
            }
            let label = request.label.trim();
            if label.is_empty()
                || label.chars().count() > 80
                || label.chars().any(char::is_control)
                || request.process_id == 0
                || request.databases.is_empty()
                || request.databases.len() > MAX_DATABASES
            {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "需要标签、明确PID和1–512个数据库",
                ));
            }
            let mut seen = BTreeSet::new();
            let mut root = None;
            let mut items = Vec::with_capacity(request.databases.len());
            for path in &request.databases {
                if path.len() > 4096
                    || path.chars().any(char::is_control)
                    || !Path::new(path).is_absolute()
                    || !seen.insert(path.to_lowercase())
                {
                    return Err(AppError::new("INVALID_ARGUMENT", "数据库路径无效或重复"));
                }
                // Provider owns path/reparse-point safety; false means the
                // client is still running, not a plan validation failure.
                self.provider.offline_ready(path)?;
                let canonical = std::fs::canonicalize(path)
                    .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
                let canonical_key = canonical.to_string_lossy().to_lowercase();
                if !seen.insert(format!("canonical:{canonical_key}")) {
                    return Err(AppError::new("INVALID_ARGUMENT", "数据库路径无效或重复"));
                }
                let selected_root = source_root(&canonical)?;
                if root.as_ref().is_some_and(|r| r != &selected_root) {
                    return Err(AppError::new(
                        "SOURCE_ROOT_MISMATCH",
                        "所选数据库不属于同一来源根目录",
                    ));
                }
                root = Some(selected_root);
                let m = std::fs::metadata(&canonical)
                    .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
                if !m.is_file() || m.len() < 4096 {
                    return Err(AppError::new(
                        "SOURCE_LIMIT_EXCEEDED",
                        "所选数据库必须为至少4KiB的普通文件",
                    ));
                }
                let deferred = m.len() > READER_MAX_BYTES;
                items.push(SetupItem {
                    path: path.clone(),
                    key_ready: false,
                    skipped: deferred,
                    source: None,
                    error: deferred.then(|| "SOURCE_LIMIT_EXCEEDED".into()),
                    outcome: Some(
                        if deferred {
                            "resource_deferred"
                        } else {
                            "pending"
                        }
                        .into(),
                    ),
                });
            }
            let fingerprint = hex::encode(Sha256::digest(
                format!("WeRC/setup-source/v1\0{}", root.unwrap()).as_bytes(),
            ));
            let installation = self.provider.inspect(&request.executable)?;
            if !werc_domain::connection::supports_config_cipher(
                &installation.version,
                &installation.architecture,
            ) {
                return Err(AppError::new(
                    "KEY_LAYOUT_UNSUPPORTED",
                    "此版本尚未适配自动取钥",
                ));
            }
            let account = self
                .repository
                .setup_account_for_source(label, &fingerprint)?;
            if let Some(old) = self
                .repository
                .bindings()?
                .iter()
                .find(|b| b.account_id == account.id)
                && (old.installation.id != installation.id
                    || old.installation.sha256 != installation.sha256)
            {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "此引导登记已绑定其他安装，请先在连接页明确解除绑定",
                ));
            }
            let binding = self.bind_client_local(&account.id, &request.executable)?;
            let mut run = SetupRun {
                id: Uuid::new_v4().to_string(),
                revision: 0,
                key_revision: self.repository.key_revision(&account.id)?,
                binding,
                process_id: request.process_id,
                phase: "keys".into(),
                items,
            };
            self.save_setup(&mut run)?;
            self.setup_active.store(true, Ordering::SeqCst);
        }
        self.setup_status_local()
    }
    pub fn setup_resume_local(
        &self,
        id: &str,
        confirmed: bool,
        skip: bool,
    ) -> AppResult<SetupStatus> {
        consent(confirmed)?;
        {
            let _guard = self.setup_guard.try_lock().map_err(|_| busy())?;
            let mut run = self
                .load_setup()?
                .ok_or_else(|| AppError::new("SETUP_NOT_FOUND", "没有待恢复引导"))?;
            if run.id != id {
                return Err(AppError::new("SETUP_CHANGED", "引导已改变"));
            }
            self.check_setup_binding(&run)?;
            if let Some(item) = run
                .items
                .iter_mut()
                .find(|i| !i.skipped && i.error.is_some())
            {
                if skip {
                    // Keep the failure evidence; skipped is not imported.
                    item.skipped = true;
                    item.outcome = Some("failed".into());
                } else {
                    item.error = None;
                    item.outcome = Some("pending".into());
                }
            }
            run.phase = if run.items.iter().any(|i| !i.key_ready && !i.skipped) {
                "keys"
            } else {
                "waiting"
            }
            .into();
            self.save_setup(&mut run)?;
            self.setup_active.store(true, Ordering::SeqCst);
        }
        self.setup_status_local()
    }
    // 每次只执行一个有界步骤；页面可轮询，但核心不会在无有效授权时继续。
    pub fn setup_step_local(&self, id: &str) -> AppResult<SetupStatus> {
        {
            let _guard = self.setup_guard.try_lock().map_err(|_| busy())?;
            if !self.setup_active.load(Ordering::SeqCst) {
                return Err(AppError::new(
                    "CONSENT_REQUIRED",
                    "引导已暂停，请重新确认继续",
                ));
            }
            let mut run = self
                .load_setup()?
                .ok_or_else(|| AppError::new("SETUP_NOT_FOUND", "没有正在执行的引导"))?;
            if run.id != id {
                return Err(AppError::new("SETUP_CHANGED", "引导已改变"));
            }
            if let Err(e) = self.check_setup_binding(&run) {
                self.setup_active.store(false, Ordering::SeqCst);
                return Err(e);
            }
            let account = run.binding.account_id.clone();
            if let Some(index) = run.items.iter().position(|i| !i.key_ready && !i.skipped) {
                match self.acquire_database_key_local(
                    &account,
                    run.process_id,
                    &run.items[index].path,
                    true,
                ) {
                    Ok(_) => {
                        run.items[index].key_ready = true;
                        run.key_revision = self.repository.key_revision(&account)?;
                    }
                    Err(e) => {
                        if settle_error(&mut run, index, e) {
                            self.setup_active.store(false, Ordering::SeqCst);
                        }
                    }
                }
                if run.phase != "failed" && run.items.iter().all(|i| i.key_ready || i.skipped) {
                    run.phase = "waiting".into();
                }
            } else if let Some(index) = run
                .items
                .iter()
                .position(|i| i.source.is_none() && !i.skipped)
            {
                let mut ready = true;
                let pending: Vec<usize> = run
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| !i.skipped && i.source.is_none())
                    .map(|(i, _)| i)
                    .collect();
                for checked in pending {
                    match self.provider.offline_ready(&run.items[checked].path) {
                        Ok(false) => {
                            run.phase = "waiting".into();
                            ready = false;
                            break;
                        }
                        Ok(true) => {}
                        Err(e) => {
                            // The checked file may not be the next import item.
                            if settle_error(&mut run, checked, e) {
                                self.setup_active.store(false, Ordering::SeqCst);
                            }
                            ready = false;
                            break;
                        }
                    }
                }
                if ready && self.setup_active.load(Ordering::SeqCst) {
                    let existing: BTreeSet<_> = self
                        .repository
                        .history_sources(&account)?
                        .into_iter()
                        .map(|s| s.id)
                        .collect();
                    run.phase = "importing".into();
                    self.save_setup(&mut run)?;
                    match self.import_history_controlled(
                        &account,
                        HistoryReadRequest {
                            path: run.items[index].path.clone(),
                            key_hex: None,
                            allow_plaintext: false,
                        },
                        true,
                        true,
                    ) {
                        Ok(source) => {
                            let outcome = if existing.contains(&source.id) {
                                "already_imported"
                            } else {
                                "imported"
                            };
                            run.items[index].source = Some(source);
                            run.items[index].error = None;
                            run.items[index].outcome = Some(outcome.into());
                        }
                        Err(e) => {
                            if settle_error(&mut run, index, e) {
                                self.setup_active.store(false, Ordering::SeqCst);
                            }
                        }
                    }
                }
            }
            if run.items.iter().all(|i| i.source.is_some() || i.skipped) {
                run.phase = if run.items.iter().any(|i| i.skipped) {
                    "partial"
                } else {
                    "complete"
                }
                .into();
                self.setup_active.store(false, Ordering::SeqCst);
                self.repository.dismiss_setup()?;
            }
            self.save_setup(&mut run)?;
        }
        self.setup_status_local()
    }
}
