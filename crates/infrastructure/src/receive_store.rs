use crate::*;
use werc_domain::receive::*;

fn cursor(c: &Connection, account: &str) -> AppResult<Option<ReceiveCursor>> {
    let value: Option<String> = c
        .query_row(
            "SELECT cursor FROM receive_streams WHERE account_id=?1",
            [account],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::internal())?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| AppError::internal()))
        .transpose()
}
impl SqliteRepository {
    pub(crate) fn native_identity_store(&self, account: &str) -> AppResult<Option<String>> {
        self.connection()?
            .query_row(
                "SELECT identity_token FROM native_identities WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())
    }
    pub(crate) fn bind_native_identity_store(
        &self,
        account: &str,
        revision: i64,
        token: &str,
    ) -> AppResult<()> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("INVALID_ARGUMENT", "身份摘要格式无效"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, account)?;
        let actual: i64 = tx
            .query_row(
                "SELECT revision FROM current_connection WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if actual != revision {
            return Err(AppError::new("CONNECTION_CHANGED", "当前配置已变化"));
        }
        tx.execute("INSERT INTO native_identities(account_id,identity_token) VALUES(?1,?2) ON CONFLICT(account_id) DO NOTHING",params![account,token]).map_err(|_|AppError::internal())?;
        let bound: String = tx
            .query_row(
                "SELECT identity_token FROM native_identities WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if bound != token {
            return Err(AppError::new(
                "NATIVE_ACCOUNT_MISMATCH",
                "当前微信登录者与此配置此前验证的身份不同；未合并资料或提交消息",
            ));
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn receive_cursor_store(&self, account: &str) -> AppResult<Option<ReceiveCursor>> {
        let c = self.connection()?;
        cursor(&c, account)
    }
    pub(crate) fn commit_receive_store(&self, data: &ReceiveCommit<'_>) -> AppResult<bool> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, data.account_id)?;
        let revision: i64 = tx
            .query_row(
                "SELECT revision FROM current_connection WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if revision != data.connection_revision
            || cursor(&tx, data.account_id)?.as_ref() != data.expected
        {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "接收连接或持久化位置已经变化；未确认消费",
            ));
        }
        let token = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !token(&data.next.epoch)
            || !token(&data.next.proof)
            || data.next.sequence > i64::MAX as u64
            || data.next.gaps > i64::MAX as u64
        {
            return Err(AppError::new("INVALID_ARGUMENT", "接收位置格式无效"));
        }
        let mut inserted = false;
        if let Some(m) = data.message {
            if m.account_id != data.account_id
                || !token(&m.id)
                || !token(&m.content_digest)
                || m.nonce.len() != 12
                || m.wrapped_key.is_empty()
                || m.wrapped_key.len() > 16384
                || m.ciphertext.len() > 131072
                || m.ciphertext.len() < 16
            {
                return Err(AppError::new("INVALID_ARGUMENT", "受保护接收消息格式无效"));
            }
            let existing: Option<String> = tx
                .query_row(
                    "SELECT content_digest FROM received_texts WHERE account_id=?1 AND id=?2",
                    params![m.account_id, m.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| AppError::internal())?;
            if existing.as_ref().is_some_and(|d| d != &m.content_digest) {
                return Err(AppError::new(
                    "RECEIVE_CONTENT_CONFLICT",
                    "同一原生标识的内容不同；未覆盖记录或确认消费",
                ));
            }
            if existing.is_none() {
                // Retained history is not a lifetime receive quota. The transaction
                // must still succeed (including disk writes) before native ACK.
                tx.execute("INSERT INTO received_texts(account_id,id,wrapped_key,nonce,ciphertext,content_digest) VALUES(?1,?2,?3,?4,?5,?6)",params![m.account_id,m.id,m.wrapped_key,m.nonce,m.ciphertext,m.content_digest]).map_err(|_|AppError::internal())?;
                inserted = true;
            }
        }
        if let Some(voice) = data.voice {
            let message = data.message.ok_or_else(AppError::internal)?;
            if voice.file.asset.account_id != data.account_id
                || voice.observation_id != message.id
                || !token(&voice.proof)
                || voice.file.asset.size > 262144
            {
                return Err(AppError::new(
                    "RECEIVE_PROTOCOL_INVALID",
                    "语音资产关联无效",
                ));
            }
            // An ACK retry reuses its already durable voice. Never overwrite it.
            let existing:Option<String>=tx.query_row("SELECT f.metadata FROM received_voice_assets v JOIN file_assets f ON f.account_id=v.account_id AND f.id=v.asset_id WHERE v.account_id=?1 AND v.observation_id=?2",params![data.account_id,voice.observation_id],|r|r.get(0)).optional().map_err(|_|AppError::internal())?;
            if let Some(metadata) = &existing {
                let asset: werc_domain::file::FileAsset =
                    serde_json::from_str(metadata).map_err(|_| AppError::internal())?;
                if asset.sha256 != voice.file.asset.sha256 {
                    return Err(AppError::new(
                        "RECEIVE_CONTENT_CONFLICT",
                        "同一语音观测的二进制内容不同；未确认消费",
                    ));
                }
            }
            if existing.is_none() {
                Self::insert_file_asset(&tx, &voice.file)?;
                tx.execute("INSERT INTO received_voice_assets(account_id,observation_id,asset_id,proof) VALUES(?1,?2,?3,?4)",params![data.account_id,voice.observation_id,voice.file.asset.id,voice.proof]).map_err(|_|AppError::internal())?;
            }
        }
        if data.expected != Some(data.next) {
            tx.execute("INSERT INTO receive_streams(account_id,cursor) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET cursor=excluded.cursor",params![data.account_id,serde_json::to_string(data.next).map_err(|_|AppError::internal())?]).map_err(|_|AppError::internal())?;
            if data.expected.is_none_or(|c| c.gaps != data.next.gaps) || inserted {
                tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'receive.updated',?2,?3)",params![data.account_id,Utc::now().to_rfc3339(),serde_json::json!({"stored":inserted,"gaps":data.next.gaps}).to_string()]).map_err(|_|AppError::internal())?;
            }
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(inserted)
    }
    pub(crate) fn received_text_by_id_store(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Option<ProtectedReceivedText>> {
        let c = self.connection()?;
        let position: Option<i64> = c
            .query_row(
                "SELECT position FROM received_texts WHERE account_id=?1 AND id=?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        drop(c);
        let Some(position) = position else {
            return Ok(None);
        };
        Ok(self
            .received_texts_store(account, position - 1, 1)?
            .into_iter()
            .find(|(p, r)| *p == position && r.id == id)
            .map(|(_, r)| r))
    }
    pub(crate) fn received_texts_store(
        &self,
        account: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<(i64, ProtectedReceivedText)>> {
        if after < -1 || !(1..=100).contains(&limit) {
            return Err(AppError::new("INVALID_ARGUMENT", "接收分页参数无效"));
        }
        let c = self.connection()?;
        let sql = if after == -1 {
            "SELECT position,id,wrapped_key,nonce,ciphertext,content_digest FROM (SELECT position,id,wrapped_key,nonce,ciphertext,content_digest FROM received_texts WHERE account_id=?1 AND position>?2 ORDER BY position DESC LIMIT ?3) ORDER BY position"
        } else {
            "SELECT position,id,wrapped_key,nonce,ciphertext,content_digest FROM received_texts WHERE account_id=?1 AND position>?2 ORDER BY position LIMIT ?3"
        };
        let mut statement = c.prepare(sql).map_err(|_| AppError::internal())?;
        statement
            .query_map(params![account, after, limit], |r| {
                if r.get_ref(2)?.as_blob()?.len() > 16384
                    || r.get_ref(3)?.as_blob()?.len() != 12
                    || r.get_ref(4)?.as_blob()?.len() > 131072
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok((
                    r.get(0)?,
                    ProtectedReceivedText {
                        account_id: account.into(),
                        id: r.get(1)?,
                        wrapped_key: r.get(2)?,
                        nonce: r.get(3)?,
                        ciphertext: r.get(4)?,
                        content_digest: r.get(5)?,
                    },
                ))
            })
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())
    }
}
