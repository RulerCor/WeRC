//! Encrypted local documents and resumable source captures. No WeChat writes.
use serde::{Deserialize, Serialize};
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentHeader {
    pub account_id: String,
    pub id: String,
    pub kind: String,
    pub revision: i64,
}
pub struct ProtectedDocument {
    pub header: DocumentHeader,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
pub struct DocumentPage {
    pub data: Vec<DocumentHeader>,
    pub next_cursor: Option<i64>,
}
/// Optional source identity guard checked in the same transaction as the documents.
pub struct DocumentConnection {
    pub revision: i64,
    pub key_revision: i64,
}
