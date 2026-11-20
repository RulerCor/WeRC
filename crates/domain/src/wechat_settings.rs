use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingKey {
    NewMessageSound,
    CallSound,
    AutoDownload,
}
impl SettingKey {
    pub const ALL: [Self; 3] = [Self::NewMessageSound, Self::CallSound, Self::AutoDownload];
    pub fn native_id(self) -> u32 {
        match self {
            Self::NewMessageSound => 1,
            Self::CallSound => 2,
            Self::AutoDownload => 3,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NewMessageSound => "新消息提示音",
            Self::CallSound => "通话提示音",
            Self::AutoDownload => "自动下载",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeBuild {
    pub version: String,
    pub architecture: String,
    pub core_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingValue {
    pub key: SettingKey,
    pub value: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub build: NativeBuild,
    pub native_epoch: String,
    pub values: Vec<SettingValue>,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingDescriptor {
    pub key: SettingKey,
    pub label: String,
    pub value_type: String,
    pub scope: String,
    pub readable: bool,
    pub writable: bool,
    pub implementation: String,
    pub requires_foreground: bool,
    pub requires_restart: bool,
    pub effect: String,
}
impl SettingDescriptor {
    pub fn native(key: SettingKey) -> Self {
        Self {
            key,
            label: key.label().into(),
            value_type: "boolean".into(),
            scope: "current_login_local".into(),
            readable: true,
            writable: true,
            implementation: "native".into(),
            requires_foreground: false,
            requires_restart: false,
            effect: match key {
                SettingKey::AutoDownload => {
                    "微信原生自动下载总开关；保留微信自身大小、网络和媒体策略，不立即下载历史文件"
                }
                _ => "微信原生提示音开关；不改变 Bot 消息流或 Windows 系统权限",
            }
            .into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsCapabilities {
    pub available: bool,
    pub build: Option<NativeBuild>,
    pub items: Vec<SettingDescriptor>,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SettingTarget {
    pub key: SettingKey,
    pub value: bool,
    pub expected_value: Option<bool>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Verified,
    Unchanged,
    Conflict,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingResult {
    pub status: ChangeStatus,
    pub before: Option<bool>,
    pub requested: bool,
    pub observed: Option<bool>,
    pub error_code: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingPlan {
    pub key: SettingKey,
    pub before: bool,
    pub requested: bool,
    pub action: String,
    pub native_epoch: String,
    pub build: NativeBuild,
    pub requires_foreground: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsChange {
    pub id: String,
    pub account_id: String,
    pub creator_id: String,
    pub idempotency_digest: String,
    pub request_digest: String,
    pub created_at: String,
    pub target: SettingTarget,
    pub native_epoch: String,
    pub build: NativeBuild,
    pub result: SettingResult,
    pub replayed: bool,
}
