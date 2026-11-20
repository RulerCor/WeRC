use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendExecutionMode {
    Native,
    VisibleAutomation,
    Synthetic,
}

// 只能由装配的可信 Provider 返回；HTTP/桌面请求不能自行声明身份已验真。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendContext {
    pub sender_id: String,
    pub target_id: String,
    pub target_display_name: String,
    pub identity_verified: bool,
    pub target_verified: bool,
    pub epoch: String,
    pub installation_sha256: String,
    pub adapter_sha256: String,
    pub execution_mode: SendExecutionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
    AwaitingApproval,
    Queued,
    Submitting,
    Unknown,
    ServerAccepted,
    RecipientConfirmed,
    Rejected,
    Canceled,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendEvidence {
    LocalAccepted,
    NativeQueued,
    ServerAccepted,
    RecipientConfirmed,
    RejectedBeforeSubmission,
    Unknown,
}

impl SendState {
    pub fn apply_evidence(self, evidence: SendEvidence) -> AppResult<Self> {
        use SendEvidence as E;
        use SendState as S;
        match (self, evidence) {
            (S::Submitting | S::Unknown, E::LocalAccepted | E::NativeQueued | E::Unknown) => {
                Ok(S::Unknown)
            }
            (S::Submitting | S::Unknown | S::ServerAccepted, E::ServerAccepted) => {
                Ok(S::ServerAccepted)
            }
            (
                S::Submitting | S::Unknown | S::ServerAccepted | S::RecipientConfirmed,
                E::RecipientConfirmed,
            ) => Ok(S::RecipientConfirmed),
            (S::Submitting | S::Unknown, E::RejectedBeforeSubmission) => Ok(S::Rejected),
            _ => Err(AppError::new(
                "INVALID_TRANSITION",
                "回执不能降级已确认结果或重新入队",
            )),
        }
    }
}

// 不含正文、收件人或真实发送者；这些字段仅在认证密文中保存。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMetadata {
    pub schema_version: u32,
    pub id: String,
    pub account_id: String,
    pub creator_id: String,
    pub idempotency_digest: String,
    pub request_digest: String,
    pub context_digest: String,
    pub connection_revision: i64,
    pub installation_sha256: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendStatus {
    pub metadata: SendMetadata,
    pub state: SendState,
    pub approval_expires_at: Option<i64>,
    #[serde(default)]
    pub scheduled_at: Option<i64>,
    pub attempt_id: Option<String>,
    pub evidence: Option<SendEvidence>,
    pub updated_at: i64,
}
#[derive(Serialize)]
pub struct SendPage {
    pub data: Vec<SendStatus>,
    pub next_cursor: Option<i64>,
}

/// Approval timing is authenticated separately from the immutable draft.
/// An absent start time retains the immediate-send contract.
pub struct SendTiming {
    pub scheduled_at: Option<i64>,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct ProtectedSend {
    pub metadata: SendMetadata,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextSendPayload {
    pub context: SendContext,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<QuoteSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moment: Option<MomentPublish>,
    // Omitted for legacy text records, preserving their authenticated JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceSendAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSendAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileSendAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoSendAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoticon: Option<EmoticonSendAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentPublish {
    pub visibility: MomentVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<MomentAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageSendAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoSendAsset>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentVisibility {
    Private,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentActionKind {
    Like,
    Comment,
    Unlike,
    Delete,
    Reply,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentReplyTarget {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id64: Option<String>,
}
impl MomentReplyTarget {
    pub fn valid(&self) -> bool {
        let number = |v: &Option<String>, max: u64| {
            v.as_ref().is_none_or(|s| {
                s.parse::<u64>()
                    .is_ok_and(|n| n > 0 && n <= max && n.to_string() == *s)
            })
        };
        !self.username.is_empty()
            && self.username.len() < 128
            && self
                .username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            && (self.id.is_some() || self.id64.is_some())
            && number(&self.id, u32::MAX as u64)
            && number(&self.id64, u64::MAX)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentAction {
    /// Decimal string preserves all 64 bits across JavaScript/JSON clients.
    pub sns_id: String,
    pub action: MomentActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<MomentReplyTarget>,
}
impl MomentAction {
    pub fn valid(&self, text: &str) -> bool {
        self.sns_id
            .parse::<u64>()
            .is_ok_and(|id| id != 0 && id.to_string() == self.sns_id)
            && if self.action == MomentActionKind::Reply {
                self.reply.as_ref().is_some_and(MomentReplyTarget::valid)
            } else {
                self.reply.is_none()
            }
            && match self.action {
                MomentActionKind::Like | MomentActionKind::Unlike | MomentActionKind::Delete => {
                    text.is_empty()
                }
                MomentActionKind::Comment => {
                    !text.trim().is_empty() && text.len() < 1024 && !text.contains('\0')
                }
                MomentActionKind::Reply => {
                    !text.trim().is_empty() && text.len() < 896 && !text.contains('\0')
                }
            }
    }
}
impl MomentPublish {
    pub fn valid(&self, text: &str) -> bool {
        match &self.interaction {
            Some(action) => {
                self.images.is_empty()
                    && self.video.is_none()
                    && self.visibility == MomentVisibility::Private
                    && action.valid(text)
            }
            None => {
                valid_moment_text(text, self.images.len())
                    && (self.video.is_none() || self.images.len() == 1)
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteSource {
    pub server_id: String,
    pub text: String,
    pub sha256: String,
}
impl QuoteSource {
    pub fn valid(&self) -> bool {
        self.server_id
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == self.server_id)
            && valid_send_text(&self.text)
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}
impl Drop for QuoteSource {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.text.zeroize();
        self.server_id.zeroize();
    }
}
pub struct QuoteSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub source: &'a QuoteSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSendAsset {
    pub asset_id: String,
    pub sha256: String,
    pub file_name: String,
    pub size: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoSendAsset {
    pub file: FileSendAsset,
    pub info: crate::video::VideoInfo,
}
pub struct VideoSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub video: &'a VideoSendAsset,
    pub bytes: &'a [u8],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmoticonSendAsset {
    pub file: FileSendAsset,
    pub info: crate::emoticon::EmoticonInfo,
}
pub struct EmoticonSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub emoticon: &'a EmoticonSendAsset,
    pub bytes: &'a [u8],
}
pub struct FileSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub file: &'a FileSendAsset,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSendAsset {
    pub asset_id: String,
    pub source_sha256: String,
    pub png_sha256: String,
    pub width: u32,
    pub height: u32,
}
pub struct ImageSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub image: &'a ImageSendAsset,
    pub png: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSendAsset {
    pub asset_id: String,
    pub source_sha256: String,
    pub silk_sha256: String,
    pub codec_sha256: String,
    pub duration_ms: u32,
}
pub struct VoiceSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub voice: &'a VoiceSendAsset,
    pub silk: &'a [u8],
}

impl Drop for TextSendPayload {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.text.zeroize();
        self.mentions.zeroize();
        if let Some(file) = &mut self.file {
            file.file_name.zeroize();
        }
        if let Some(video) = &mut self.video {
            video.file.file_name.zeroize();
        }
        if let Some(e) = &mut self.emoticon {
            e.file.file_name.zeroize();
        }
        self.context.sender_id.zeroize();
        self.context.target_id.zeroize();
        self.context.target_display_name.zeroize();
        self.context.epoch.zeroize();
    }
}

// Provider 必须原子核对上下文 epoch/目标/摘要并在批准期限内提交。
// 入队/本地受理不是服务器接受；不支持可靠回查时返回 Unknown，绝不自动重发。
pub struct TextSubmission<'a> {
    pub outbox_id: &'a str,
    pub attempt_id: &'a str,
    pub request_digest: &'a str,
    pub context_digest: &'a str,
    pub context: &'a SendContext,
    pub text: &'a str,
    pub deadline_unix: i64,
}

// 回查不是再次提交，不携带正文，也不复用已经到期的提交期限。
pub struct MentionSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub members: &'a [String],
}

pub struct TextReceiptQuery<'a> {
    pub outbox_id: &'a str,
    pub attempt_id: &'a str,
    pub context_digest: &'a str,
    pub context: &'a SendContext,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SendReceipt {
    pub outbox_id: String,
    pub attempt_id: String,
    pub context_digest: String,
    pub evidence: SendEvidence,
    pub provider_reference: Option<String>,
}

/// Identity captured from the exact native send task, not inferred from message content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTextIdentity {
    pub local_id: u32,
    pub timestamp: i64,
}
impl NativeTextIdentity {
    const PREFIX: &str = "native-text-local:v1:";
    pub fn reference(&self) -> String {
        format!("{}{}/{}", Self::PREFIX, self.local_id, self.timestamp)
    }
    pub fn parse(reference: &str) -> Option<Self> {
        let (local, timestamp) = reference.strip_prefix(Self::PREFIX)?.split_once('/')?;
        let value = Self {
            local_id: local.parse().ok()?,
            timestamp: timestamp.parse().ok()?,
        };
        (value.local_id > 0
            && value.timestamp > 0
            && value.timestamp <= u32::MAX as i64
            && value.reference() == reference)
            .then_some(value)
    }
}

/// Causal native task identity for ordinary messages. Legacy text proofs decode
/// as type 1 so existing encrypted outboxes remain queryable after upgrades.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMessageIdentity {
    pub local_id: u32,
    pub timestamp: i64,
    #[serde(default = "default_native_message_type")]
    pub message_type: u64,
}
fn default_native_message_type() -> u64 {
    1
}
impl NativeMessageIdentity {
    pub fn valid(&self) -> bool {
        self.local_id > 0
            && self.timestamp > 0
            && self.timestamp <= u32::MAX as i64
            && [1, 3, 34, 43, 47, (6u64 << 32) | 49, (57u64 << 32) | 49]
                .contains(&self.message_type)
    }
    pub fn reference(&self) -> String {
        if self.message_type == 1 {
            return NativeTextIdentity {
                local_id: self.local_id,
                timestamp: self.timestamp,
            }
            .reference();
        }
        format!(
            "native-message-local:v1:{}/{}/{}",
            self.local_id, self.timestamp, self.message_type
        )
    }
    pub fn parse(reference: &str) -> Option<Self> {
        if let Some(text) = NativeTextIdentity::parse(reference) {
            return Some(Self {
                local_id: text.local_id,
                timestamp: text.timestamp,
                message_type: 1,
            });
        }
        let fields: Vec<_> = reference
            .strip_prefix("native-message-local:v1:")?
            .split('/')
            .collect();
        if fields.len() != 3 {
            return None;
        }
        let value = Self {
            local_id: fields[0].parse().ok()?,
            timestamp: fields[1].parse().ok()?,
            message_type: fields[2].parse().ok()?,
        };
        (value.valid() && value.reference() == reference).then_some(value)
    }
}

pub struct ProtectedReceipt {
    pub outbox_id: String,
    pub attempt_id: String,
    pub context_digest: String,
    pub evidence: SendEvidence,
    pub protected: Vec<u8>,
}

#[derive(Serialize)]
pub struct SendPreview {
    pub schedule_before: Option<i64>,
    pub local_record: Option<LocalRecordStatus>,
    pub status: SendStatus,
    pub payload: TextSendPayload,
}

/// Independent observation of the WeChat local record. Missing evidence cannot
/// establish a write failure and is never used to retry an external send.
#[derive(Serialize)]
pub struct LocalRecordStatus {
    pub state: &'static str,
    pub observed_at: Option<i64>,
}

/// A locally selected recipient. None expiry is reserved for built-in endpoints.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PreparedRecipient {
    pub target_id: String,
    pub display_name: String,
    pub expires_at: Option<i64>,
}

/// Native text limit is UTF-8 bytes, independent of Unicode character count.
pub const MAX_TEXT_BYTES: usize = 10_000;
pub fn valid_send_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES && !text.contains('\0')
}

pub fn valid_moment_text(text: &str, image_count: usize) -> bool {
    image_count <= 9 && (valid_send_text(text) || (text.is_empty() && image_count > 0))
}
pub struct MomentImage<'a> {
    pub asset: &'a ImageSendAsset,
    pub png: &'a [u8],
}
pub struct MomentImageSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub images: &'a [MomentImage<'a>],
}
pub struct MomentVideoSubmission<'a> {
    pub submission: TextSubmission<'a>,
    pub cover: MomentImage<'a>,
    pub video: &'a VideoSendAsset,
    pub bytes: &'a [u8],
}

#[cfg(test)]
mod moment_tests {
    use super::*;
    #[test]
    fn private_image_limits_and_old_text_serialization_are_compatible() {
        for count in 0..=10 {
            assert_eq!(valid_moment_text("", count), (1..=9).contains(&count));
            assert_eq!(valid_moment_text(&"中".repeat(3333), count), count <= 9);
            assert!(!valid_moment_text(&"中".repeat(3334), count));
            assert!(!valid_moment_text("a\0b", count));
        }
        let old = r#"{"visibility":"private"}"#;
        let parsed: MomentPublish = serde_json::from_str(old).unwrap();
        assert!(parsed.images.is_empty());
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);
    }
}

#[cfg(test)]
mod native_identity_tests {
    use super::*;
    #[test]
    fn native_message_identity_binds_exact_type_and_reads_legacy_text_proofs() {
        for message_type in [1, 3, 34, 43, 47, (6u64 << 32) | 49, (57u64 << 32) | 49] {
            let id = NativeMessageIdentity {
                local_id: 487,
                timestamp: 1700000000,
                message_type,
            };
            assert_eq!(NativeMessageIdentity::parse(&id.reference()), Some(id));
        }
        let old: NativeMessageIdentity =
            serde_json::from_str(r#"{"local_id":487,"timestamp":1700000000}"#).unwrap();
        assert_eq!(old.message_type, 1);
        for bad in [
            "native-message-local:v1:487/1700000000/1",
            "native-message-local:v1:0487/1700000000/3",
            "native-message-local:v1:0/1700000000/3",
            "native-message-local:v1:487/1700000000/49",
        ] {
            assert!(NativeMessageIdentity::parse(bad).is_none());
        }
    }
}
