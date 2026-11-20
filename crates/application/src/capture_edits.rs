//! Previewed, revision-checked changes to encrypted WeRC copies only.
use super::*;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePatch {
    #[serde(default)]
    pub extra: Option<CaptureFields>,
    pub text: Option<String>,
    pub timestamp: i64,
    pub message_type: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFields {
    pub username: String,
    pub sender_username: Option<String>,
    pub server_id: Option<String>,
    pub metadata: Option<String>,
}
fn edit_operation() -> String {
    "edit".into()
}
#[derive(Serialize, Deserialize)]
struct Change {
    #[serde(default = "edit_operation")]
    operation: String,
    capture_id: String,
    page: u64,
    row: usize,
    page_revision: i64,
    before: CaptureRow,
    after: CaptureRow,
    state: String,
    created_at: String,
}
#[derive(Serialize)]
pub struct ChangePreview {
    pub operation: String,
    pub id: String,
    pub capture_id: String,
    pub page: u64,
    pub row: usize,
    pub state: String,
    pub before: CapturePatch,
    pub after: CapturePatch,
    pub created_at: String,
}
fn fields(row: &CaptureRow) -> CapturePatch {
    CapturePatch {
        extra: Some(CaptureFields {
            username: row.username.clone(),
            sender_username: row.message.sender_username.clone(),
            server_id: row.message.server_id.clone(),
            metadata: row.message.metadata.clone(),
        }),
        text: row.message.text.clone(),
        timestamp: row.message.timestamp,
        message_type: row.message.message_type.clone(),
    }
}
fn preview(h: &DocumentHeader, c: &Change) -> ChangePreview {
    ChangePreview {
        operation: c.operation.clone(),
        id: h.id.clone(),
        capture_id: c.capture_id.clone(),
        page: c.page,
        row: c.row,
        state: c.state.clone(),
        before: fields(&c.before),
        after: fields(&c.after),
        created_at: c.created_at.clone(),
    }
}
impl Application {
    pub fn preview_capture_edit_local(
        &self,
        account: &str,
        id: &str,
        index: u64,
        row: usize,
        revision: i64,
        patch: CapturePatch,
    ) -> AppResult<ChangePreview> {
        if !(0..253402300800).contains(&patch.timestamp)
            || patch.message_type.parse::<i64>().is_err()
            || patch
                .text
                .as_ref()
                .is_some_and(|s| s.len() > 16384 || s.contains('\0'))
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "修改内容、类型或时间无效",
            ));
        }
        if let Some(extra) = &patch.extra {
            validate_fields(extra)?;
        }
        let (_, capture): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if index >= capture.pages {
            return Err(AppError::new("NOT_FOUND", "保存页不存在"));
        }
        let (h, batch): (_, CaptureBatch) =
            self.open_document(account, &format!("{id}:{index}"), "capture_page")?;
        if h.revision != revision {
            return Err(AppError::new(
                "DOCUMENT_CONFLICT",
                "此页已更新，请重新加载后修改",
            ));
        }
        let before = batch
            .rows
            .get(row)
            .ok_or_else(|| AppError::new("NOT_FOUND", "消息不存在"))?
            .clone();
        let mut after = before.clone();
        if before.message.content_status == "deleted_local" {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "请先恢复已删除的本地记录",
            ));
        }
        if let Some(extra) = patch.extra {
            if after.username != extra.username || after.message.sender_username != extra.sender_username || after.message.server_id != extra.server_id {
                after.message.at_user_list = None;
            }
            after.username = extra.username;
            after.message.sender_username = extra.sender_username;
            after.message.server_id = extra.server_id;
            after.message.metadata = extra.metadata;
        }
        after.message.text = patch.text;
        after.message.timestamp = patch.timestamp;
        after.message.message_type = patch.message_type;
        after.app_subtype=capture_subtype(&after.message);
        after.message.content_status = "edited_local".into();
        let change = Change {
            operation: "edit".into(),
            capture_id: id.into(),
            page: index,
            row,
            page_revision: h.revision,
            before,
            after,
            state: "pending".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let doc = self.seal_document(
            account,
            &uuid::Uuid::new_v4().to_string(),
            "capture_change",
            1,
            &change,
        )?;
        let output = preview(&doc.header, &change);
        self.repository.library_save(&[doc], None)?;
        Ok(output)
    }
    pub fn apply_capture_edit_local(
        &self,
        account: &str,
        id: &str,
        action: &str,
    ) -> AppResult<ChangePreview> {
        let (h, mut change): (_, Change) = self.open_document(account, id, "capture_change")?;
        if action == "cancel" && change.state == "pending" {
            change.state = "canceled".into();
            let doc = self.seal_document(account, id, "capture_change", h.revision + 1, &change)?;
            let result = preview(&doc.header, &change);
            self.repository.library_save(&[doc], None)?;
            return Ok(result);
        }
        let revert = action == "restore" && change.state == "applied";
        if !revert && !(action == "apply" && change.state == "pending") {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "此变更不能执行当前操作",
            ));
        }
        if change.operation == "add" && !revert {
            let (ch, mut capture): (_, Capture) =
                self.open_document(account, &change.capture_id, "source_capture")?;
            if capture.pages != change.page || ch.revision != change.page_revision {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "归档进度已变化，请重新预览添加",
                ));
            }
            let page_id = format!("{}:{}", change.capture_id, change.page);
            let page = CaptureBatch {
                capture_id: change.capture_id.clone(),
                index: change.page,
                rows: vec![change.after.clone()],
            };
            capture.pages += 1;
            capture.rows += 1;
            change.state = "applied".into();
            // New page starts at revision 1; its undo checks that exact revision.
            change.page_revision = 0;
            let d = self.seal_document(account, id, "capture_change", h.revision + 1, &change)?;
            let result = preview(&d.header, &change);
            self.repository.library_save(
                &[
                    self.seal_document(account, &page_id, "capture_page", 1, &page)?,
                    self.seal_document(
                        account,
                        &change.capture_id,
                        "source_capture",
                        ch.revision + 1,
                        &capture,
                    )?,
                    d,
                ],
                None,
            )?;
            return Ok(result);
        }
        let page_id = format!("{}:{}", change.capture_id, change.page);
        let (page_header, mut page): (_, CaptureBatch) =
            self.open_document(account, &page_id, "capture_page")?;
        let expected = change.page_revision + if revert { 1 } else { 0 };
        if page_header.revision != expected {
            return Err(AppError::new(
                "DOCUMENT_CONFLICT",
                "此页已有后续变更；未覆盖现有资料",
            ));
        }
        let row = page
            .rows
            .get_mut(change.row)
            .ok_or_else(|| AppError::new("DOCUMENT_CORRUPTED", "变更消息不存在"))?;
        *row = if revert {
            let mut before = change.before.clone();
            if change.operation == "add" {
                before = change.after.clone();
                before.message.content_status = "deleted_local".into();
            }
            before
        } else {
            change.after.clone()
        };
        change.state = if revert { "restored" } else { "applied" }.into();
        let page_doc = self.seal_document(
            account,
            &page_id,
            "capture_page",
            page_header.revision + 1,
            &page,
        )?;
        let change_doc =
            self.seal_document(account, id, "capture_change", h.revision + 1, &change)?;
        let result = preview(&change_doc.header, &change);
        self.repository
            .library_save(&[page_doc, change_doc], None)?;
        Ok(result)
    }
    pub fn capture_edits_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<ChangePreview>, Option<i64>)> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "capture_change", before, 20)?;
        let values = page
            .data
            .into_iter()
            .map(|h| {
                self.open_document::<Change>(account, &h.id, "capture_change")
                    .map(|(h, c)| preview(&h, &c))
            })
            .collect::<AppResult<_>>()?;
        Ok((values, page.next_cursor))
    }
}

fn validate_fields(fields: &CaptureFields) -> AppResult<()> {
    if !werc_domain::live::valid_live_username(&fields.username)
        || fields
            .sender_username
            .as_deref()
            .is_some_and(|v| !werc_domain::live::valid_live_username(v))
        || fields
            .server_id
            .as_ref()
            .is_some_and(|v| v.parse::<u64>().is_err())
        || fields
            .metadata
            .as_ref()
            .is_some_and(|v| v.len() > 65536 || v.contains('\0'))
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "来源、发送者、编号或原始内容无效",
        ));
    }
    Ok(())
}
impl Application {
    pub fn capture_edit_fields_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
    ) -> AppResult<CapturePatch> {
        let (_, r) = self.capture_media_row_for_edit(account, id, page, row)?;
        Ok(fields(&r))
    }
    fn capture_media_row_for_edit(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
    ) -> AppResult<(DocumentHeader, CaptureRow)> {
        let (_, c): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if page >= c.pages {
            return Err(AppError::new("NOT_FOUND", "归档页不存在"));
        }
        let (h, b): (_, CaptureBatch) =
            self.open_document(account, &format!("{id}:{page}"), "capture_page")?;
        Ok((
            h,
            b.rows
                .get(row)
                .ok_or_else(|| AppError::new("NOT_FOUND", "消息不存在"))?
                .clone(),
        ))
    }
    pub fn preview_capture_delete_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
        revision: i64,
    ) -> AppResult<ChangePreview> {
        let (h, before) = self.capture_media_row_for_edit(account, id, page, row)?;
        if h.revision != revision {
            return Err(AppError::new("DOCUMENT_CONFLICT", "归档页已变化"));
        }
        let kind = before.message.message_type.parse::<i64>().unwrap_or(0) & 0xffffffff;
        if !matches!(kind, 10000 | 10002) || before.message.content_status == "deleted_local" {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "此入口仅删除本地副本中的系统消息",
            ));
        }
        let mut after = before.clone();
        after.message.content_status = "deleted_local".into();
        let change = Change {
            operation: "delete_system".into(),
            capture_id: id.into(),
            page,
            row,
            page_revision: revision,
            before,
            after,
            state: "pending".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let d = self.seal_document(
            account,
            &uuid::Uuid::new_v4().to_string(),
            "capture_change",
            1,
            &change,
        )?;
        let result = preview(&d.header, &change);
        self.repository.library_save(&[d], None)?;
        Ok(result)
    }
    pub fn preview_capture_add_local(
        &self,
        account: &str,
        id: &str,
        username: &str,
        text: &str,
        timestamp: i64,
    ) -> AppResult<ChangePreview> {
        if !werc_domain::live::valid_live_username(username)
            || text.trim().is_empty()
            || text.len() > 16384
            || text.contains('\0')
            || !(0..253402300800).contains(&timestamp)
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "本地新增记录的会话、时间或文字无效",
            ));
        }
        let (h, c): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if !c.imported && c.source_index < c.sources.len() {
            return Err(AppError::new("BUSY", "请先完成补读，再添加离线记录"));
        }
        let after = CaptureRow {
            app_subtype:None,
            username: username.into(),
            source_index: 0,
            message: LiveMessage {
                at_user_list: None,
                local_id: format!("local-{}", uuid::Uuid::new_v4()),
                server_id: None,
                sender_username: None,
                timestamp,
                message_type: "1".into(),
                text: Some(text.into()),
                metadata: None,
                content_status: "added_local".into(),
            },
        };
        let mut before = after.clone();
        before.message.text = None;
        before.message.content_status = "absent_local".into();
        let change = Change {
            operation: "add".into(),
            capture_id: id.into(),
            page: c.pages,
            row: 0,
            page_revision: h.revision,
            before,
            after,
            state: "pending".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let d = self.seal_document(
            account,
            &uuid::Uuid::new_v4().to_string(),
            "capture_change",
            1,
            &change,
        )?;
        let result = preview(&d.header, &change);
        self.repository.library_save(&[d], None)?;
        Ok(result)
    }
}

impl Application {
    /// Explicitly restore a row to the earliest applied change's authenticated
    /// before-image. Unrelated later page edits still require a fresh preview.
    pub fn preview_capture_original_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
        revision: i64,
    ) -> AppResult<ChangePreview> {
        let (header, current) = self.capture_media_row_for_edit(account, id, page, row)?;
        if header.revision != revision {
            return Err(AppError::new("DOCUMENT_CONFLICT", "归档页已变化"));
        }
        let mut cursor = None;
        let mut original: Option<(i64, CaptureRow, String)> = None;
        loop {
            let changes = self
                .repository
                .library_list(account, "capture_change", cursor, 100)?;
            for h in changes.data {
                let (_, c): (_, Change) = self.open_document(account, &h.id, "capture_change")?;
                if c.capture_id == id
                    && c.page == page
                    && c.row == row
                    && matches!(c.state.as_str(), "applied" | "restored" | "archived")
                    && original.as_ref().is_none_or(|o| c.page_revision < o.0)
                {
                    original = Some((c.page_revision, c.before, c.operation));
                }
            }
            cursor = changes.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let (_, after, operation) =
            original.ok_or_else(|| AppError::new("NOT_FOUND", "此消息没有已应用的本地修改"))?;
        if operation == "add" {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "本地新增记录没有微信原文，请从变更记录撤销添加",
            ));
        }
        if serde_json::to_vec(&current).map_err(|_| AppError::internal())?
            == serde_json::to_vec(&after).map_err(|_| AppError::internal())?
        {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "此消息已经是保存的原始内容",
            ));
        }
        let change = Change {
            operation: "restore_original".into(),
            capture_id: id.into(),
            page,
            row,
            page_revision: revision,
            before: current,
            after,
            state: "pending".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let d = self.seal_document(
            account,
            &uuid::Uuid::new_v4().to_string(),
            "capture_change",
            1,
            &change,
        )?;
        let result = preview(&d.header, &change);
        self.repository.library_save(&[d], None)?;
        Ok(result)
    }
}
