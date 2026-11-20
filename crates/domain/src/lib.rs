pub mod special_records;
pub mod wechat_settings;
use serde::{Deserialize, Serialize};

pub mod asr;
pub mod articles;
pub mod browse;
pub mod capability;
pub mod connection;
pub mod current;
pub mod emoticon;
pub mod favorites;
pub mod file;
pub mod group;
pub mod history;
pub mod image;
pub mod library;
pub mod live;
mod live_metadata;
pub mod media;
pub mod mentions;
pub mod message_card;
pub mod moments;
pub mod onboarding;
pub mod operation;
pub mod outbox;
pub mod projection;
pub mod receive;
pub mod received_emoticon;
pub mod received_media;
pub mod recovery;
pub mod video;
pub mod voice;
pub mod wxgf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub state: AccountState,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Registered,
    NeedsKey,
    Ready,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn internal() -> Self {
        Self::new("INTERNAL_ERROR", "内部存储或服务错误；请检查本机诊断")
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: i64,
    pub schema_version: u32,
    pub event_type: String,
    pub account_id: Option<String>,
    pub occurred_at: String,
    pub observed_at: String,
    pub payload: serde_json::Value,
}

/// Reserved expiration value; preserves compatibility with existing SQLite and JSON data.
pub const PERMANENT_TOKEN_EXPIRY: &str = "9999-12-31T23:59:59+00:00";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub id: String,
    pub label: String,
    pub account_ids: Vec<String>,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    pub revoked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IssuedToken {
    pub info: TokenInfo,
    pub secret: String,
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub account_ids: Vec<String>,
    pub scopes: Vec<String>,
}

impl Principal {
    pub fn require(&self, scope: &str) -> AppResult<()> {
        if !self.scopes.iter().any(|s| s == scope) {
            return Err(AppError::new("FORBIDDEN", "令牌没有此操作的权限"));
        }
        Ok(())
    }

    pub fn require_account(&self, account_id: &str) -> AppResult<()> {
        if !self.account_ids.iter().any(|id| id == account_id) {
            return Err(AppError::new("FORBIDDEN", "令牌未获准访问此账号"));
        }
        Ok(())
    }
}

pub mod article_history;
