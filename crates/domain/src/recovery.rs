//! Durable source observations for gap investigation; never hook acknowledgements.
use crate::{live::LiveMessage, live::LiveWindow, receive::ReceiveCursor};
use serde::{Deserialize, Serialize};

pub const MAX_RECOVERY_WINDOWS: usize = 128;
pub const MAX_RECOVERY_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct RecoverySummary {
    pub id: String,
    pub captured_at: String,
    pub rows: usize,
    pub text_rows: usize,
    pub has_older: bool,
    pub gaps_at_capture: Option<u64>,
    pub proof: String,
}
pub struct ProtectedRecovery {
    pub account_id: String,
    pub summary: RecoverySummary,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
pub struct RecoveryCommit<'a> {
    pub account_id: &'a str,
    pub connection_revision: i64,
    pub key_revision: i64,
    pub expected_receive: Option<&'a ReceiveCursor>,
    pub record: &'a ProtectedRecovery,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPayload {
    pub schema_version: u32,
    pub account_id: String,
    pub id: String,
    /// Keyed source path/salt/login scope. New captures also bind Windows file
    /// identity. Older captures stay readable; neither proves full coverage.
    pub source_scope: String,
    pub captured_at: String,
    pub receive_cursor: Option<ReceiveCursor>,
    pub window: LiveWindow,
}
#[derive(Serialize)]
pub struct RecoveryPage {
    pub summary: RecoverySummary,
    pub rows: Vec<LiveMessage>,
    /// Parallel to rows; source numbers are local to this saved window.
    pub source_indices: Vec<usize>,
    pub source_count: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub mode: &'static str,
    pub gaps_closed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiRecoveryRow {
    pub source_index: usize,
    pub message: LiveMessage,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiRecoveryWindow {
    pub rows: Vec<MultiRecoveryRow>,
    /// Each source's keyed path/salt/file/login scope, in canonical order.
    pub sources: Vec<String>,
    pub has_older: Vec<bool>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiRecoveryPayload {
    pub schema_version: u32,
    pub account_id: String,
    pub id: String,
    pub captured_at: String,
    pub receive_cursor: Option<ReceiveCursor>,
    pub window: MultiRecoveryWindow,
}
