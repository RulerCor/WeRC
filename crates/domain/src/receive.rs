//! Observations from an authenticated native stream. These are not delivery
//! receipts and do not imply that disconnected intervals have been backfilled.
use crate::outbox::SendContext;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Clone, Serialize)]
pub struct ReceiveHealth {
    pub connected: bool,
    pub error_code: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedText {
    pub sequence: u64,
    pub native_message_id: String,
    pub text: String,
    #[serde(default = "text_type", skip_serializing_if = "is_text_type")]
    pub message_type: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}
fn text_type() -> u32 {
    1
}
fn is_text_type(v: &u32) -> bool {
    *v == 1
}
pub fn observed_type(v: u32) -> bool {
    matches!(v, 1 | 3 | 34 | 37 | 42 | 43 | 47 | 48 | 49 | 50 | 62 | 10000 | 10002)
}
impl ReceivedText {
    pub fn valid_content(&self) -> bool {
        let content = if self.message_type == 1 {
            if self.metadata.is_some() {
                return false;
            }
            &self.text
        } else {
            if !self.text.is_empty() || !observed_type(self.message_type) {
                return false;
            }
            let Some(metadata) = &self.metadata else {
                return false;
            };
            metadata
        };
        !content.is_empty() && content.len() <= 16383 && !content.contains('\0')
    }
}
impl Drop for ReceivedText {
    fn drop(&mut self) {
        self.text.zeroize();
        self.metadata.zeroize();
        self.native_message_id.zeroize();
    }
}
pub struct ReceiveBatch {
    /// Native envelope bound to the authenticated login. None is legacy filehelper only.
    pub route: Option<ReceivedRoute>,
    /// Owned inline type34 bytes; never serialized in message listings.
    pub inline_voice: Option<zeroize::Zeroizing<Vec<u8>>>,
    pub context: SendContext,
    pub dropped: u32,
    pub message: Option<ReceivedText>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiveCursor {
    pub proof: String,
    pub last_id: Option<String>,
    /// Keyed identity/epoch token; no account identifier from Weixin on disk.
    pub epoch: String,
    pub sequence: u64,
    pub dropped: u32,
    pub gaps: u64,
}
pub struct ProtectedReceivedText {
    pub account_id: String,
    pub id: String,
    pub content_digest: String,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
pub struct ProtectedReceivedVoice {
    pub file: crate::file::ProtectedFile,
    pub observation_id: String,
    pub proof: String,
}
pub struct ReceiveCommit<'a> {
    pub voice: Option<&'a ProtectedReceivedVoice>,
    pub account_id: &'a str,
    pub connection_revision: i64,
    pub expected: Option<&'a ReceiveCursor>,
    pub next: &'a ReceiveCursor,
    pub message: Option<&'a ProtectedReceivedText>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedTextPayload {
    pub schema_version: u32,
    pub account_id: String,
    pub id: String,
    /// Authenticated local login identity; not the sender of the observed item.
    pub sender_id: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<ReceivedRoute>,
    pub observed_at: String,
    pub message: ReceivedText,
}

/// A group envelope identifies the group, not the individual member author.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedRoute {
    pub sender: String,
    pub receiver: String,
    pub conversation: String,
}
impl ReceivedRoute {
    pub fn valid_for(&self, login: &str) -> bool {
        fn id(value: &str) -> bool {
            !value.is_empty()
                && value.len() < 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@'))
        }
        id(login)
            && id(&self.sender)
            && id(&self.receiver)
            && id(&self.conversation)
            && if self.sender == login {
                self.conversation == self.receiver
            } else {
                self.receiver == login && self.conversation == self.sender
            }
    }
}
impl Drop for ReceivedRoute {
    fn drop(&mut self) {
        self.sender.zeroize();
        self.receiver.zeroize();
        self.conversation.zeroize();
    }
}
#[derive(Serialize)]
pub struct ReceivedTextRow {
    /// Product-local page position, not a Weixin server sequence.
    pub position: i64,
    pub payload: ReceivedTextPayload,
    #[serde(skip_serializing_if="Option::is_none")]
    pub card: Option<crate::message_card::DisplayContent>,
}
#[derive(Serialize)]
pub struct ReceiveProgress {
    pub stored: bool,
    pub acknowledged: bool,
    pub gaps: u64,
}
