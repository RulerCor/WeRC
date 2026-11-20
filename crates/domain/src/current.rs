use serde::{Deserialize, Serialize};

// 本地隔离槽位，不是微信登录身份认证结果；64位修订号经字符串传递。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrentConnection {
    pub account_id: Option<String>,
    pub revision: String,
    pub selection_required: bool,
    pub identity_verified: bool,
}
