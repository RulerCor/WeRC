//! Explicit durable copies of already authenticated bounded source observations.
//! No source read, stream ACK, gap closure, ID alias or send reconciliation here.
use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use werc_domain::{AppError, AppResult, live::*, recovery::*};
use zeroize::Zeroizing;

pub(crate) fn corrupt() -> AppError {
    AppError::new("RECOVERY_CORRUPTED", "补读记录认证失败")
}
fn scope(account: &str, id: &str, proof: &str) -> String {
    format!("WeRC/source-recovery/v1/{account}/{id}/{proof}")
}
pub(crate) struct RecoveryCapture<'a> {
    pub account: &'a str,
    pub revision: i64,
    pub key_revision: i64,
    pub cursor: Option<&'a werc_domain::receive::ReceiveCursor>,
}
impl Application {
    pub(crate) fn recovery_proof(&self, account: &str, s: &RecoverySummary) -> AppResult<String> {
        self.send_digest(
            account,
            "source-recovery-summary-v1",
            &(
                &s.id,
                &s.captured_at,
                s.rows,
                s.text_rows,
                s.has_older,
                s.gaps_at_capture,
            ),
        )
    }
    fn check_recovery_summary(&self, account: &str, s: &RecoverySummary) -> AppResult<()> {
        if s.id.len() != 64
            || s.rows > LIVE_WINDOW_MAX_ROWS
            || s.text_rows > s.rows
            || self.recovery_proof(account, s)? != s.proof
        {
            return Err(corrupt());
        }
        Ok(())
    }
    pub fn recover_live_filehelper_local(
        &self,
        account: &str,
        view_id: &str,
        confirmed: bool,
    ) -> AppResult<RecoverySummary> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需要确认保存当前在线窗口为加密补读记录",
            ));
        }
        let _history = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let _receive = self
            .receive_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        let result = slot
            .as_ref()
            .filter(|r| r.account == account)
            .ok_or_else(|| AppError::new("CURSOR_STALE", "请重新读取在线窗口"))?;
        let (binding, salt, source) = self.live_read_binding(account, &result.database)?;
        if binding != result.binding {
            return Err(AppError::new("CURSOR_STALE", "来源或身份已变化"));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        let cursor = self.receive_cursor_local(account)?;
        let window = result.view.recovery_window(&binding, view_id)?;
        let canonical = std::fs::canonicalize(&*result.database)
            .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
        let source_scope = self.send_digest(
            account,
            "source-recovery-scope-v2",
            &(
                &context.sender_id,
                &context.target_id,
                canonical.to_string_lossy().to_lowercase(),
                salt,
                &source,
            ),
        )?;
        // Same content may be saved again after reconnect without duplicating it.
        // Local/server IDs remain observations within this window, never aliases.
        let id = self.send_digest(
            account,
            "source-recovery-window-v1",
            &(&source_scope, &window),
        )?;
        let mut summary = RecoverySummary {
            id: id.clone(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            rows: window.rows.len(),
            text_rows: window.rows.iter().filter(|r| r.text.is_some()).count(),
            has_older: window.has_older,
            gaps_at_capture: cursor.as_ref().map(|c| c.gaps),
            proof: String::new(),
        };
        summary.proof = self.recovery_proof(account, &summary)?;
        let payload = RecoveryPayload {
            schema_version: 1,
            account_id: account.into(),
            id,
            source_scope,
            captured_at: summary.captured_at.clone(),
            receive_cursor: cursor.clone(),
            window,
        };
        self.store_recovery_payload(
            RecoveryCapture {
                account,
                revision,
                key_revision,
                cursor: cursor.as_ref(),
            },
            summary,
            &payload,
            || {
                if self.live_read_binding(account, &result.database)?.0 != binding
                    || result.view.expired()
                    || self.provider.resolve_send_context(account, "filehelper")? != context
                {
                    return Err(AppError::new(
                        "CONNECTION_CHANGED",
                        "保存期间身份或来源已变化",
                    ));
                }
                Ok(())
            },
        )
    }
    pub(crate) fn store_recovery_payload<T: serde::Serialize>(
        &self,
        capture: RecoveryCapture<'_>,
        summary: RecoverySummary,
        payload: &T,
        revalidate: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<RecoverySummary> {
        let account = capture.account;
        let plain = Zeroizing::new(serde_json::to_vec(payload).map_err(|_| AppError::internal())?);
        if plain.len() > LIVE_WINDOW_MAX_BYTES + 16384 {
            return Err(AppError::new("QUOTA_EXCEEDED", "补读窗口超过保护上限"));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = scope(account, &summary.id, &summary.proof);
        let record = ProtectedRecovery {
            account_id: account.into(),
            summary,
            ciphertext: Aes256Gcm::new_from_slice(key.as_ref())
                .map_err(|_| corrupt())?
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &plain,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| corrupt())?,
            wrapped_key: self.provider.protect_key(&aad, key.as_ref())?,
            nonce: nonce.to_vec(),
        };
        revalidate()?;
        let saved = self.repository.commit_recovery(&RecoveryCommit {
            account_id: account,
            connection_revision: capture.revision,
            key_revision: capture.key_revision,
            expected_receive: capture.cursor,
            record: &record,
        })?;
        if saved.id != record.summary.id {
            return Err(corrupt());
        }
        self.check_recovery_summary(account, &saved)?;
        // Includes the idempotent-existing path: never report a damaged saved
        // envelope as a successful recovery merely because its ID still exists.
        self.recovered_filehelper_page_local(account, &saved.id, 0, 1)?;
        Ok(saved)
    }
    pub fn recovery_summaries_local(&self, account: &str) -> AppResult<Vec<RecoverySummary>> {
        let summaries = self.repository.recovery_summaries(account)?;
        for s in &summaries {
            self.check_recovery_summary(account, s)?;
        }
        Ok(summaries)
    }
    pub fn recovered_filehelper_page_local(
        &self,
        account: &str,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> AppResult<RecoveryPage> {
        if !(1..=100).contains(&limit) || offset > LIVE_WINDOW_MAX_ROWS {
            return Err(AppError::new("INVALID_ARGUMENT", "补读分页参数无效"));
        }
        let r = self.repository.recovery(account, id)?;
        if r.account_id != account || r.summary.id != id || r.nonce.len() != 12 {
            return Err(corrupt());
        }
        self.check_recovery_summary(account, &r.summary)?;
        let aad = scope(account, id, &r.summary.proof);
        let key = Zeroizing::new(self.provider.unprotect_key(&aad, &r.wrapped_key)?);
        let plain = Zeroizing::new(
            Aes256Gcm::new_from_slice(&key)
                .map_err(|_| corrupt())?
                .decrypt(
                    Nonce::from_slice(&r.nonce),
                    Payload {
                        msg: &r.ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| corrupt())?,
        );
        #[derive(serde::Deserialize)]
        struct Version {
            schema_version: u32,
        }
        let version: Version = serde_json::from_slice(&plain).map_err(|_| corrupt())?;
        if version.schema_version == 2 {
            return self.multi_recovery_page(account, r.summary, &plain, offset, limit);
        }
        let p: RecoveryPayload = serde_json::from_slice(&plain).map_err(|_| corrupt())?;
        if p.schema_version != 1
            || p.account_id != account
            || p.id != id
            || p.captured_at != r.summary.captured_at
            || p.window.rows.len() != r.summary.rows
            || p.window.rows.iter().filter(|r| r.text.is_some()).count() != r.summary.text_rows
            || p.window.has_older != r.summary.has_older
            || p.receive_cursor.as_ref().map(|c| c.gaps) != r.summary.gaps_at_capture
            || self.send_digest(
                account,
                "source-recovery-window-v1",
                &(&p.source_scope, &p.window),
            )? != id
        {
            return Err(corrupt());
        }
        if offset > p.window.rows.len() {
            return Err(AppError::new("INVALID_ARGUMENT", "补读位置超出窗口"));
        }
        let end = (offset + limit).min(p.window.rows.len());
        Ok(RecoveryPage {
            summary: r.summary,
            rows: p.window.rows[offset..end].to_vec(),
            source_count: 1,
            source_indices: vec![1; end - offset],
            offset,
            next_offset: (end < p.window.rows.len()).then_some(end),
            mode: "saved_source_observation",
            gaps_closed: false,
        })
    }
}
