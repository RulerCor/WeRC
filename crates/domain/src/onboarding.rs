use crate::connection::ClientBinding;
use crate::history::HistorySource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    pub label: String,
    pub executable: String,
    pub process_id: u32,
    pub databases: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SetupItem {
    pub path: String,
    pub key_ready: bool,
    pub skipped: bool,
    pub source: Option<HistorySource>,
    pub error: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SetupRun {
    pub id: String,
    pub revision: u64,
    pub binding: ClientBinding,
    pub process_id: u32,
    pub key_revision: i64,
    pub phase: String,
    pub items: Vec<SetupItem>,
}

#[derive(Clone, Serialize)]
pub struct SetupStatus {
    pub run: Option<SetupRun>,
    pub active: bool,
    pub working: bool,
    pub dismissed: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ProtectedSetup {
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
