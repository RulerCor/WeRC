use serde::{Deserialize, Serialize};

/// Address-independent WCDB key discovery; page authentication remains mandatory.
/// This capability does not imply that the native messaging ABI is supported.
pub fn supports_config_cipher(version: &str, architecture: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    architecture == "x86_64"
        && parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && parts[0] == "4"
        && matches!(parts[1], "1" | "2")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientInstallation {
    pub id: String,
    pub executable_path: String,
    pub version: String,
    pub architecture: String,
    pub product_name: String,
    pub sha256: String,
    pub process_ids: Vec<u32>,
    pub sources: Vec<String>,
    pub compatibility: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub installations: Vec<ClientInstallation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientBinding {
    pub account_id: String,
    pub installation: ClientInstallation,
    pub bound_at: String,
    pub checked_at: String,
    pub validation: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataSourceCatalog {
    pub sources: Vec<DataSourceCandidate>,
    pub warnings: Vec<String>,
    pub roots_checked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceCandidate {
    pub id: String,
    pub account_directory: String,
    pub directory_name: String,
    pub layout: String,
    pub identity_verified: bool,
    pub databases: Vec<DatabaseFile>,
    pub has_media_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseFile {
    pub relative_path: String,
    pub category: String,
    pub size: u64,
    pub wal_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStatus {
    pub account_id: String,
    pub protected: bool,
    pub verified: bool,
    pub verified_databases: usize,
    pub verification_error: Option<String>,
    pub updated_at: Option<String>,
}

// 原始候选仅限核心与本机Provider之间传递，禁止序列化或Debug输出。
pub struct AcquiredDatabaseKey {
    pub key: zeroize::Zeroizing<[u8; 32]>,
    pub salt: [u8; 16],
    pub first_page_sha256: [u8; 32],
}
/// Secret material stays off serializable provider/status types.
pub struct ImageCacheKey {pub aes:Option<zeroize::Zeroizing<[u8;16]>>,pub xor:Option<u8>}
