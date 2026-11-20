mod asr;
pub mod articles;
pub mod article_jobs;
pub mod backfill;
pub mod capture_tools;
pub mod capture_timeline;
pub mod capture_social;
mod connections;
pub use connections::RevealedDatabaseKey;
pub mod contact_avatars;
pub mod conversation_card;
pub mod conversation_media;
mod current;
mod wechat_settings;
mod account_removal;
pub mod directory_export;
mod emoticon_send;
mod exact_receipts;
pub mod export_plans;
mod file_media;
mod file_send;
mod history;
mod history_export;
mod history_stream;
mod image_media;
pub mod image_cache_keys;
mod image_send;
mod keyring;
pub mod library;
mod library_backup;
pub mod live_calendar;
pub mod live_contacts;
pub mod live_conversation;
mod live_emoticon;
pub mod live_export;
pub mod live_favorites;
pub mod live_groups;
pub mod live_moments;
mod live_multi;
pub mod live_multi_conversation;
pub mod live_multi_view;
mod live_read;
pub mod live_search;
pub mod live_sessions;
pub mod live_special_records;
pub mod live_view;
mod live_voice;
mod media;
pub mod mention_send;
pub mod moment_preservation;
pub mod message_preservation;
mod moment_send;
mod onboarding;
mod outbox;
pub mod ports;
pub mod quote_send;
pub mod receipt_sweep;
mod receive;
mod received_emoticon;
mod received_media;
mod received_video;
pub mod recipients;
mod recovery;
mod recovery_multi;
mod send_wait;
pub mod source_capture;
mod tasks;
mod source_gate;
mod video_send;
use chrono::{Datelike, Duration, Utc};
use ports::{CapabilityProvider, Repository};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;
use werc_domain::capability::Capability;
use werc_domain::{Account, AppError, AppResult, Event, IssuedToken, Principal, TokenInfo};

pub const SCOPES: &[&str] = &[
    "wechat:settings:read",
    "wechat:settings:write",
    "accounts:read",
    "connections:read",
    "sources:read",
    "contacts:read",
    "group-names:read",
    "friends:read",
    "directory:read",
    "sessions:read",
    "moments:read",
    "exports:create",
    "media:read",
    "media:write",
    "operations:read",
    "operations:create",
    "messages:send_voice",
    "messages:send_image",
    "messages:send_file",
    "messages:send_video",
    "messages:send_emoticon",
    "capabilities:read",
    "events:read",
    "messages:read",
    "messages:send",
    "messages:execute",
    "moments:publish",
    "openapi:read",
];

#[derive(Debug, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub product: String,
    pub version: String,
    pub stage: String,
    pub api_enabled: bool,
    pub real_wechat_verified: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenValidity {
    Timed { seconds: i64 },
    Permanent,
}

fn invalid_token_duration() -> AppError {
    AppError::new("INVALID_ARGUMENT", "有效期需要为正数，且到期时间不能超出可表示范围")
}

pub struct Application {
    repository: Arc<dyn Repository>,
    provider: Arc<dyn CapabilityProvider>,
    api_enabled: AtomicBool,
    history_guard: source_gate::SourceGate,
    projection_guard: std::sync::Mutex<()>,
    setup_guard: std::sync::Mutex<()>,
    setup_active: AtomicBool,
    send_guard: std::sync::Mutex<()>,
    settings_guard: std::sync::Mutex<()>,
    receive_guard: std::sync::Mutex<()>,
    backfill_guard: std::sync::Mutex<()>,
    message_preservation_guard: std::sync::Mutex<()>,
    moment_preservation_guard: std::sync::Mutex<()>,
    backfill_discovery: std::sync::Mutex<backfill::DiscoveryState>,
    received_media_guard: std::sync::Mutex<()>,
    conversation_result: std::sync::Mutex<Option<live_conversation::ConversationResult>>,
    conversation_generation: std::sync::atomic::AtomicU64,
    multi_conversation_result:
        std::sync::Mutex<Option<live_multi_conversation::MultiConversationResult>>,
    multi_conversation_generation: std::sync::atomic::AtomicU64,
    live_result: std::sync::Mutex<Option<live_read::LiveResult>>,
    multi_live_result: std::sync::Mutex<Option<live_multi::MultiResult>>,
    browse_jobs: std::sync::Mutex<std::collections::HashMap<String, history::QueryJob>>,
    browse_canceled: std::sync::Mutex<std::collections::VecDeque<String>>,
}

pub fn token_digest(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

impl Application {
    pub fn new(repository: Arc<dyn Repository>, provider: Arc<dyn CapabilityProvider>) -> Self {
        Self {
            repository,
            provider,
            api_enabled: AtomicBool::new(false),
            history_guard: source_gate::SourceGate::default(),
            projection_guard: std::sync::Mutex::new(()),
            setup_guard: std::sync::Mutex::new(()),
            setup_active: AtomicBool::new(false),
            send_guard: std::sync::Mutex::new(()),
            settings_guard: std::sync::Mutex::new(()),
            receive_guard: std::sync::Mutex::new(()),
            backfill_guard: std::sync::Mutex::new(()),
            message_preservation_guard: std::sync::Mutex::new(()),
            moment_preservation_guard: std::sync::Mutex::new(()),
            backfill_discovery: std::sync::Mutex::new(Default::default()),
            received_media_guard: std::sync::Mutex::new(()),
            conversation_result: std::sync::Mutex::new(None),
            conversation_generation: std::sync::atomic::AtomicU64::new(0),
            multi_conversation_result: std::sync::Mutex::new(None),
            multi_conversation_generation: std::sync::atomic::AtomicU64::new(0),
            live_result: std::sync::Mutex::new(None),
            multi_live_result: std::sync::Mutex::new(None),
            browse_jobs: std::sync::Mutex::new(std::collections::HashMap::new()),
            browse_canceled: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            product: "WeRC_Mon3trBot".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            stage: "stable".into(),
            api_enabled: self.api_enabled.load(Ordering::SeqCst),
            real_wechat_verified: false,
        }
    }

    pub fn enable_api(&self, enabled: bool) {
        self.api_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn authenticate(&self, secret: &str) -> AppResult<Principal> {
        if !self.api_enabled.load(Ordering::SeqCst) {
            return Err(AppError::new("API_PAUSED", "本机已暂停 API 访问"));
        }
        if !(32..=256).contains(&secret.len()) {
            return Err(AppError::new("UNAUTHORIZED", "令牌无效或已撤销"));
        }
        self.repository.authenticate(&token_digest(secret))
    }

    pub fn accounts_local(&self) -> AppResult<Vec<Account>> {
        let mut accounts = self.repository.accounts()?;
        for account in &mut accounts {
            account.state = self.account_state(&account.id)?;
        }
        Ok(accounts)
    }

    pub fn accounts(&self, principal: &Principal) -> AppResult<Vec<Account>> {
        principal.require("accounts:read")?;
        Ok(self
            .accounts_local()?
            .into_iter()
            .filter(|a| principal.account_ids.contains(&a.id))
            .collect())
    }

    pub fn register_account_local(&self, label: &str) -> AppResult<Account> {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 80 || label.chars().any(char::is_control) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "账号标签需要 1–80 个非控制字符",
            ));
        }
        self.repository.register_account(label)
    }

    pub fn tokens_local(&self) -> AppResult<Vec<TokenInfo>> {
        self.repository.tokens()
    }
    pub fn revoke_token_local(&self, id: &str) -> AppResult<()> {
        self.repository.revoke_token(id)
    }

    pub fn issue_token_local(
        &self,
        label: &str,
        account_ids: Vec<String>,
        scopes: Vec<String>,
        valid_days: i64,
    ) -> AppResult<IssuedToken> {
        let seconds = valid_days.checked_mul(86_400).ok_or_else(invalid_token_duration)?;
        self.issue_token_with_validity_local(label, account_ids, scopes, TokenValidity::Timed { seconds })
    }

    pub fn issue_token_with_validity_local(
        &self,
        label: &str,
        account_ids: Vec<String>,
        scopes: Vec<String>,
        validity: TokenValidity,
    ) -> AppResult<IssuedToken> {
        if label.trim().is_empty()
            || label.chars().count() > 80
            || label.chars().any(char::is_control)
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "令牌名称需要 1–80 个非控制字符",
            ));
        }
        if account_ids.is_empty()
            || scopes.is_empty()
            || scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "需要明确账号和有效权限",
            ));
        }
        let current = self.current_connection_local()?;
        if account_ids.len() != 1 || current.account_id.as_ref() != account_ids.first() {
            return Err(AppError::new(
                "CURRENT_CONNECTION_REQUIRED",
                "令牌只能授权唯一当前微信，不能授权归档资料或多个范围",
            ));
        }
        let existing = self.repository.accounts()?;
        if account_ids
            .iter()
            .any(|id| !existing.iter().any(|a| &a.id == id))
        {
            return Err(AppError::new("ACCOUNT_NOT_FOUND", "授权账号不存在"));
        }
        let now = Utc::now();
        let expires_at = match validity {
            TokenValidity::Permanent => werc_domain::PERMANENT_TOKEN_EXPIRY.to_owned(),
            TokenValidity::Timed { seconds } => Duration::try_seconds(seconds)
                .filter(|_| seconds > 0)
                .and_then(|duration| now.checked_add_signed(duration))
                .filter(|expiry| expiry.year() < 10_000)
                .map(|expiry| expiry.to_rfc3339())
                .filter(|expiry| expiry.as_str() < werc_domain::PERMANENT_TOKEN_EXPIRY)
                .ok_or_else(invalid_token_duration)?,
        };
        let info = TokenInfo {
            id: Uuid::new_v4().to_string(),
            label: label.trim().into(),
            account_ids,
            scopes,
            created_at: now.to_rfc3339(),
            expires_at,
            revoked: false,
        };
        let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.repository
            .create_token(&info, &token_digest(&secret))?;
        Ok(IssuedToken { info, secret })
    }

    pub fn capabilities(&self) -> Vec<Capability> {
        self.provider.capabilities()
    }

    pub fn events(
        &self,
        principal: &Principal,
        account_id: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<Event>> {
        principal.require("events:read")?;
        principal.require_account(account_id)?;
        if after < 0 || !(1..=200).contains(&limit) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "事件游标必须非负，页大小为 1–200",
            ));
        }
        self.repository.events(after, account_id, limit)
    }

    pub fn require_wechat_capability(
        &self,
        principal: &Principal,
        account_id: &str,
        scope: &str,
        capability: &str,
    ) -> AppResult<()> {
        principal.require(scope)?;
        principal.require_account(account_id)?;
        let implemented = self.provider.capabilities().into_iter().any(|c| {
            c.id == capability && c.availability == werc_domain::capability::Availability::Ready
        });
        if !implemented {
            return Err(AppError::new(
                "CAPABILITY_NOT_IMPLEMENTED",
                "此微信能力尚未实现并验证；没有提交任何操作",
            ));
        }
        Ok(())
    }
}

pub mod moment_media;

pub mod moment_bundle;

pub mod article_images;
mod article_bundle;
