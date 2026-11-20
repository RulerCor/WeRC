use crate::history::{HistoryQuery, HistorySource};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    Realtime,
    OnDemandHistory,
    ImportedSnapshot,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseDirection {
    Latest,
    Older,
    Newer,
    Around,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowseRequest {
    pub request_id: String,
    pub mode: ReadMode,
    pub source_id: Option<String>,
    pub direction: BrowseDirection,
    pub anchor_id: Option<String>,
    pub anchor_timestamp: Option<i64>,
    pub query: HistoryQuery,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadContext {
    pub mode: ReadMode,
    pub account_id: String,
    pub connection_revision: String,
    pub generation: String,
    pub observed_at: String,
    pub sources: Vec<HistorySource>,
    pub identity_status: String,
    pub query_strategy: String,
    pub query_metrics: Option<QueryMetrics>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMetrics {
    pub authenticated_batches: u64,
    pub elapsed_ms: u64,
    pub projection_built: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageView {
    pub schema_version: u32,
    pub id: String,
    pub conversation_id: String,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub direction: String,
    pub identity_status: String,
    pub server_id: String,
    pub local_id: String,
    pub sequence: String,
    pub timestamp: i64,
    pub message_type: String,
    pub text: Option<String>,
    pub content_status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelinePage {
    pub data: Vec<MessageView>,
    pub older_cursor: Option<String>,
    pub newer_cursor: Option<String>,
    pub context: ReadContext,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationView {
    pub id: String,
    pub display_name: String,
    pub identity_status: String,
    pub summary: Option<String>,
    pub timestamp: i64,
    pub historical_unread_count: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPage {
    pub data: Vec<ConversationView>,
    pub next_cursor: Option<String>,
    pub context: ReadContext,
}
