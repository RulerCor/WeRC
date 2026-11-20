use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub id: String,
    pub conversation_id: String,
    pub sender_id: Option<String>,
    pub local_id: i64,
    // 以字符串保存64位服务端ID，避免JavaScript精度丢失。
    pub server_id: String,
    pub timestamp: i64,
    pub sequence: i64,
    pub message_type: i64,
    pub text: Option<String>,
    pub content_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryContact {
    pub username: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySession {
    pub username: String,
    pub summary: Option<String>,
    pub timestamp: i64,
    pub unread_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryBundle {
    pub schema: String,
    pub source_sha256: String,
    pub encryption: String,
    pub wal_committed_frames: usize,
    pub messages: Vec<HistoryMessage>,
    pub contacts: Vec<HistoryContact>,
    pub sessions: Vec<HistorySession>,
    pub warnings: Vec<String>,
}

pub const HISTORY_BATCH_BYTES: usize = 2 * 1024 * 1024;
pub const HISTORY_BATCH_ROWS: usize = 500;
// Canonical message IDs may grow by at most 64 bytes per wire record.
pub const HISTORY_STORED_BATCH_BYTES: usize = HISTORY_BATCH_BYTES + HISTORY_BATCH_ROWS * 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryReadEvent {
    Begin {
        schema: String,
        source_sha256: String,
        encryption: String,
        wal_committed_frames: usize,
    },
    Batch {
        sequence: u64,
        digest: String,
        data: HistoryBundle,
    },
    End {
        batches: u64,
        message_count: usize,
        contact_count: usize,
        session_count: usize,
    },
    Error {
        code: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySource {
    pub id: String,
    pub account_id: String,
    pub source_sha256: String,
    pub schema: String,
    pub encryption: String,
    pub message_count: usize,
    pub contact_count: usize,
    pub session_count: usize,
    pub imported_at: String,
    pub real_wechat_verified: bool,
}

// 只允许本机调用方提交；不实现Debug/Serialize以避免意外记录密钥和来源路径。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryReadRequest {
    pub path: String,
    pub key_hex: Option<String>,
    #[serde(default)]
    pub allow_plaintext: bool,
}
impl Drop for HistoryReadRequest {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(key) = &mut self.key_hex {
            key.zeroize();
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryChange {
    pub message_id: String,
    pub change: &'static str,
}

pub struct ProtectedHistory {
    pub version: u32,
    pub changes: Vec<HistoryChange>,
    pub source: HistorySource,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub struct HistoryMessageState {
    pub source_id: String,
    pub digest: String,
}

pub struct HistoryRecordDigest {
    pub message_id: String,
    pub digest: String,
}

pub struct ProtectedHistoryBatch {
    pub sequence: u64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub records: Vec<HistoryRecordDigest>,
    pub changes: Vec<HistoryChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryQuery {
    pub conversation_id: Option<String>,
    pub sender_id: Option<String>,
    pub message_type: Option<i64>,
    pub search: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
    pub cursor: Option<String>,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePage {
    pub data: Vec<HistoryMessage>,
    pub next_cursor: Option<String>,
    pub source_ids: Vec<String>,
    pub data_mode: String,
}
