//! Persistent multi-source observations, independent of live view epochs.
use crate::{
    Application,
    recovery::{RecoveryCapture, corrupt},
};
use std::collections::BTreeSet;
use werc_domain::{AppError, AppResult, live::*, recovery::*};

impl Application {
    pub fn recover_live_filehelper_multi_local(
        &self,
        account: &str,
        view_id: &str,
        confirmed: bool,
    ) -> AppResult<RecoverySummary> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需要确认保存当前多分片窗口",
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
        let slot = self
            .multi_live_result
            .lock()
            .map_err(|_| AppError::internal())?;
        let result = slot
            .as_ref()
            .filter(|r| r.account == account)
            .ok_or_else(|| AppError::new("CURSOR_STALE", "请重新读取多分片窗口"))?;
        let binding = self.multi_context(account, &result.sources)?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        let cursor = self.receive_cursor_local(account)?;
        let (rows, has_older) = result.view.recovery_window(&binding, view_id)?;
        let mut sources = Vec::new();
        for source in &result.sources {
            let (current, salt, identity) = self.live_read_binding(account, &source.database)?;
            if current != source.binding {
                return Err(corrupt());
            }
            let canonical = std::fs::canonicalize(&*source.database)
                .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
            sources.push(self.send_digest(
                account,
                "source-recovery-scope-v2",
                &(
                    &context.sender_id,
                    &context.target_id,
                    canonical.to_string_lossy().to_lowercase(),
                    salt,
                    identity,
                ),
            )?);
        }
        let mut window = MultiRecoveryWindow {
            rows: rows
                .into_iter()
                .map(|r| MultiRecoveryRow {
                    source_index: r.source_index,
                    message: r.message,
                })
                .collect(),
            sources,
            has_older,
        };
        let mut order: Vec<_> = window.sources.iter().cloned().enumerate().collect();
        order.sort_by(|a, b| a.1.cmp(&b.1));
        let mut indices = vec![0; order.len()];
        for (new, (old, _)) in order.iter().enumerate() {
            indices[*old] = new + 1;
        }
        for row in &mut window.rows {
            row.source_index = *indices
                .get(row.source_index.wrapping_sub(1))
                .ok_or_else(corrupt)?;
        }
        window.has_older = order
            .iter()
            .map(|(old, _)| window.has_older[*old])
            .collect();
        window.sources = order.into_iter().map(|(_, source)| source).collect();
        // Persist stable source order, not ephemeral live-view tags. This makes
        // identical windows deduplicate after reconnect or source reordering.
        window.rows.sort_by_key(|r| std::cmp::Reverse(row_key(r)));
        validate_window(&window)?;
        let id = self.send_digest(account, "source-recovery-multi-window-v2", &window)?;
        let mut summary = RecoverySummary {
            id: id.clone(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            rows: window.rows.len(),
            text_rows: window
                .rows
                .iter()
                .filter(|r| r.message.text.is_some())
                .count(),
            has_older: window.has_older.iter().any(|v| *v),
            gaps_at_capture: cursor.as_ref().map(|c| c.gaps),
            proof: String::new(),
        };
        summary.proof = self.recovery_proof(account, &summary)?;
        let payload = MultiRecoveryPayload {
            schema_version: 2,
            account_id: account.into(),
            id,
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
                if self.multi_context(account, &result.sources)? != binding
                    || result.view.expired()
                    || self.provider.resolve_send_context(account, "filehelper")? != context
                {
                    return Err(AppError::new(
                        "CONNECTION_CHANGED",
                        "保存期间身份或分片来源已变化",
                    ));
                }
                Ok(())
            },
        )
    }
    pub(crate) fn multi_recovery_page(
        &self,
        account: &str,
        summary: RecoverySummary,
        plain: &[u8],
        offset: usize,
        limit: usize,
    ) -> AppResult<RecoveryPage> {
        let p: MultiRecoveryPayload = serde_json::from_slice(plain).map_err(|_| corrupt())?;
        if p.schema_version != 2
            || p.account_id != account
            || p.id != summary.id
            || p.captured_at != summary.captured_at
            || p.window.rows.len() != summary.rows
            || p.window
                .rows
                .iter()
                .filter(|r| r.message.text.is_some())
                .count()
                != summary.text_rows
            || p.window.has_older.iter().any(|v| *v) != summary.has_older
            || p.receive_cursor.as_ref().map(|c| c.gaps) != summary.gaps_at_capture
            || self.send_digest(account, "source-recovery-multi-window-v2", &p.window)?
                != summary.id
        {
            return Err(corrupt());
        }
        validate_window(&p.window)?;
        if offset > p.window.rows.len() {
            return Err(AppError::new("INVALID_ARGUMENT", "补读位置超出窗口"));
        }
        let end = (offset + limit).min(p.window.rows.len());
        Ok(RecoveryPage {
            summary,
            rows: p.window.rows[offset..end]
                .iter()
                .map(|r| r.message.clone())
                .collect(),
            source_indices: p.window.rows[offset..end]
                .iter()
                .map(|r| r.source_index)
                .collect(),
            source_count: p.window.sources.len(),
            offset,
            next_offset: (end < p.window.rows.len()).then_some(end),
            mode: "saved_source_observation",
            gaps_closed: false,
        })
    }
}
fn row_key(row: &MultiRecoveryRow) -> (i64, usize, i64) {
    (
        row.message.timestamp,
        row.source_index,
        row.message.local_id.parse().unwrap_or(0),
    )
}
fn validate_window(window: &MultiRecoveryWindow) -> AppResult<()> {
    if !(1..=crate::live_multi_view::MAX_SOURCES).contains(&window.sources.len())
        || window.has_older.len() != window.sources.len()
        || window.rows.len() > LIVE_WINDOW_MAX_ROWS
        || window.sources.iter().collect::<BTreeSet<_>>().len() != window.sources.len()
        || window
            .sources
            .iter()
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        || window
            .rows
            .iter()
            .any(|r| r.source_index == 0 || r.source_index > window.sources.len())
        || window
            .rows
            .windows(2)
            .any(|p| row_key(&p[0]) <= row_key(&p[1]))
    {
        return Err(corrupt());
    }
    for i in 1..=window.sources.len() {
        // Reuse strict single-source content/ID/order validation; an exhausted
        // source may have no rows while another source has older data remaining.
        let observed = LiveWindow {
            schema_version: 1,
            rows: window
                .rows
                .iter()
                .filter(|r| r.source_index == i)
                .map(|r| r.message.clone())
                .collect(),
            has_older: false,
            read_only: true,
            readonly_shm: true,
            read_mode: "bounded_transaction_window".into(),
        };
        drop(crate::live_view::encode_window(&observed)?);
    }
    Ok(())
}
