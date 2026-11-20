// 可重建缓存，不含明文联系人、会话或消息定位。原历史格式不变。
pub const PROJECTION_BYTES: usize = 256 * 1024 * 1024;
pub const PROJECTION_HEADER_BYTES: usize = 16 * 1024 * 1024;
pub const PROJECTION_ROW_BYTES: usize = 16 * 1024 * 1024;

pub struct ProtectedProjection {
    pub id: String,
    pub account_id: String,
    pub generation: String,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
pub struct ProtectedProjectionRow {
    pub token: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
