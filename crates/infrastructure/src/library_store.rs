use crate::*;
use werc_domain::library::*;

impl SqliteRepository {
    pub(crate) fn library_import_store(
        &self,
        account: &str,
        documents: &mut dyn Iterator<Item = AppResult<ProtectedDocument>>,
    ) -> AppResult<u64> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let mut count = 0;
        for document in documents {
            let d = document?;
            let h = &d.header;
            if h.account_id != account
                || h.revision != 1
                || h.id.is_empty()
                || h.id.len() > 128
                || h.kind.len() > 32
                || d.nonce.len() != 12
                || d.wrapped_key.is_empty()
                || d.wrapped_key.len() > 16384
                || d.ciphertext.len() > MAX_DOCUMENT_BYTES + 16
            {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "导入资料格式无效"));
            }
            tx.execute("INSERT INTO library_documents(account_id,id,kind,revision,wrapped_key,nonce,ciphertext) VALUES(?1,?2,?3,1,?4,?5,?6)",params![account,h.id,h.kind,d.wrapped_key,d.nonce,d.ciphertext]).map_err(|_|AppError::new("DOCUMENT_CONFLICT","导入资料标识重复；本次导入已回滚"))?;
            count += 1;
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(count)
    }
    pub(crate) fn library_save_store(
        &self,
        documents: &[ProtectedDocument],
        connection: Option<&DocumentConnection>,
    ) -> AppResult<()> {
        if documents.is_empty() || documents.len() > 3 {
            return Err(AppError::new("INVALID_ARGUMENT", "资料批次无效"));
        }
        let account = &documents[0].header.account_id;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        if let Some(expected) = connection {
            crate::current_store::require_current(&tx, account)?;
            let revision: i64 = tx
                .query_row(
                    "SELECT revision FROM current_connection WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| AppError::internal())?;
            let key:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[account],|r|r.get(0)).map_err(|_|AppError::internal())?;
            if revision != expected.revision || key != expected.key_revision {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "连接或来源密钥已变化；进度未推进",
                ));
            }
        }
        for d in documents {
            let h = &d.header;
            if &h.account_id != account
                || h.revision <= 0
                || h.id.is_empty()
                || h.id.len() > 128
                || h.kind.is_empty()
                || h.kind.len() > 32
                || d.nonce.len() != 12
                || d.wrapped_key.is_empty()
                || d.wrapped_key.len() > 16384
                || !(16..=MAX_DOCUMENT_BYTES + 16).contains(&d.ciphertext.len())
            {
                return Err(AppError::new("INVALID_ARGUMENT", "资料保护格式无效"));
            }
            let count=if h.revision==1 {
                tx.execute("INSERT OR IGNORE INTO library_documents(account_id,id,kind,revision,wrapped_key,nonce,ciphertext) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![account,h.id,h.kind,h.revision,d.wrapped_key,d.nonce,d.ciphertext])
            } else {
                tx.execute("UPDATE library_documents SET revision=?4,wrapped_key=?5,nonce=?6,ciphertext=?7 WHERE account_id=?1 AND id=?2 AND kind=?3 AND revision=?8",params![account,h.id,h.kind,h.revision,d.wrapped_key,d.nonce,d.ciphertext,h.revision-1])
            }.map_err(|_|AppError::internal())?;
            if count != 1 {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "资料已被更新，请重新加载",
                ));
            }
            if h.kind == "backfill_page" && h.revision == 1 {
                tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'receive.recovered',?2,?3)", params![account, Utc::now().to_rfc3339(), serde_json::json!({"page_id":h.id,"mode":"source_recovery","native_gaps_closed":false}).to_string()]).map_err(|_|AppError::internal())?;
            }
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn library_get_store(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<ProtectedDocument> {
        self.connection()?.query_row("SELECT kind,revision,wrapped_key,nonce,ciphertext FROM library_documents WHERE account_id=?1 AND id=?2 AND length(ciphertext)<=4194320 AND length(wrapped_key)<=16384 AND length(nonce)=12",params![account,id],|r|Ok(ProtectedDocument{header:DocumentHeader{account_id:account.into(),id:id.into(),kind:r.get(0)?,revision:r.get(1)?},wrapped_key:r.get(2)?,nonce:r.get(3)?,ciphertext:r.get(4)?})).optional().map_err(|_|AppError::internal())?.ok_or_else(||AppError::new("NOT_FOUND","资料不存在或格式无效"))
    }
    pub(crate) fn library_list_store(
        &self,
        account: &str,
        kind: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<DocumentPage> {
        if !(1..=100).contains(&limit) || before.is_some_and(|v| v <= 0) {
            return Err(AppError::new("INVALID_ARGUMENT", "资料分页参数无效"));
        }
        let c = self.connection()?;
        let mut q=c.prepare("SELECT position,id,revision,kind FROM library_documents WHERE account_id=?1 AND (?2='*' OR kind=?2) AND (?3 IS NULL OR position<?3) ORDER BY position DESC LIMIT ?4").map_err(|_|AppError::internal())?;
        let mut rows = q
            .query_map(params![account, kind, before, limit + 1], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    DocumentHeader {
                        account_id: account.into(),
                        id: r.get(1)?,
                        kind: r.get(3)?,
                        revision: r.get(2)?,
                    },
                ))
            })
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = if more { rows.last().map(|r| r.0) } else { None };
        Ok(DocumentPage {
            data: rows.into_iter().map(|r| r.1).collect(),
            next_cursor,
        })
    }
    pub(crate) fn library_delete_store(
        &self,
        account: &str,
        id: &str,
        revision: i64,
    ) -> AppResult<()> {
        if self
            .connection()?
            .execute(
                "DELETE FROM library_documents WHERE account_id=?1 AND id=?2 AND revision=?3",
                params![account, id, revision],
            )
            .map_err(|_| AppError::internal())?
            != 1
        {
            return Err(AppError::new("DOCUMENT_CONFLICT", "资料已被更新或移除"));
        }
        Ok(())
    }
}
