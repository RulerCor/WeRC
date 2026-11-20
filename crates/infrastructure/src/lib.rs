mod connection_store;
mod current_store;
#[cfg(test)]
mod current_tests;
mod file_store;
mod history_store;
mod history_stream_store;
mod image_store;
mod library_store;
mod media_store;
mod migration_backup;
mod onboarding_store;
mod outbox_store;
mod projection_store;
mod receive_store;
mod recovery_store;
pub mod storage_usage;
mod task_store;
mod settings_store;
mod video_store;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;
use werc_application::ports::Repository;
use werc_domain::{Account, AccountState, AppError, AppResult, Event, Principal, TokenInfo};

pub struct SqliteRepository {
    connection: Mutex<Connection>,
}

impl SqliteRepository {
    pub fn open(path: &Path) -> AppResult<Self> {
        let connection = Connection::open(path).map_err(|_| AppError::internal())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| AppError::internal())?;
        migration_backup::before_upgrade(&connection, path)?;
        Self::initialize(connection)
    }

    pub fn in_memory() -> AppResult<Self> {
        Self::initialize(Connection::open_in_memory().map_err(|_| AppError::internal())?)
    }

    fn initialize(mut connection: Connection) -> AppResult<Self> {
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| AppError::internal())?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|_| AppError::internal())?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| AppError::internal())?;
        let migrations = [
            include_str!("../migrations/0001.sql"),
            include_str!("../migrations/0002.sql"),
            include_str!("../migrations/0003.sql"),
            include_str!("../migrations/0004.sql"),
            include_str!("../migrations/0005.sql"),
            include_str!("../migrations/0006.sql"),
            include_str!("../migrations/0007.sql"),
            include_str!("../migrations/0008.sql"),
            include_str!("../migrations/0009.sql"),
            include_str!("../migrations/0010.sql"),
            include_str!("../migrations/0011.sql"),
            include_str!("../migrations/0012.sql"),
            include_str!("../migrations/0013.sql"),
            include_str!("../migrations/0014.sql"),
            include_str!("../migrations/0015.sql"),
            include_str!("../migrations/0016.sql"),
            include_str!("../migrations/0017.sql"),
            include_str!("../migrations/0018.sql"),
            include_str!("../migrations/0019.sql"),
            include_str!("../migrations/0020.sql"),
            include_str!("../migrations/0021.sql"),
            include_str!("../migrations/0022.sql"),
        ];
        if version < 0 || version as usize > migrations.len() {
            return Err(AppError::new(
                "STORE_VERSION_UNSUPPORTED",
                "产品数据库由更高版本创建",
            ));
        }
        for (index, migration) in migrations.iter().enumerate() {
            let tx = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| AppError::internal())?;
            let current: i64 = tx
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(|_| AppError::internal())?;
            if current == index as i64 {
                tx.execute_batch(migration)
                    .map_err(|_| AppError::internal())?;
            }
            tx.commit().map_err(|_| AppError::internal())?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| AppError::internal())
    }
}

impl Repository for SqliteRepository {
    fn settings_change_by_key(&self,a:&str,c:&str,d:&str)->AppResult<Option<werc_domain::wechat_settings::SettingsChange>>{self.find_settings_key(a,c,d)}
    fn settings_change_save(&self,v:&werc_domain::wechat_settings::SettingsChange)->AppResult<()>{self.save_settings_change(v)}
    fn settings_changes(&self,a:&str,b:Option<i64>,l:u32)->AppResult<Vec<(i64,werc_domain::wechat_settings::SettingsChange)>>{self.page_settings_changes(a,b,l)}
    fn settings_change(&self,a:&str,id:&str)->AppResult<Option<werc_domain::wechat_settings::SettingsChange>>{self.find_settings_change(a,id)}
    fn library_import(
        &self,
        account: &str,
        documents: &mut dyn Iterator<Item = AppResult<werc_domain::library::ProtectedDocument>>,
    ) -> AppResult<u64> {
        self.library_import_store(account, documents)
    }
    fn library_save(
        &self,
        documents: &[werc_domain::library::ProtectedDocument],
        connection: Option<&werc_domain::library::DocumentConnection>,
    ) -> AppResult<()> {
        self.library_save_store(documents, connection)
    }
    fn library_get(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::library::ProtectedDocument> {
        self.library_get_store(account, id)
    }
    fn library_count(&self, account: &str, kind: &str) -> AppResult<u64> {
        self.connection()?
            .query_row(
                "SELECT COUNT(*) FROM library_documents WHERE account_id=?1 AND kind=?2",
                params![account, kind],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())
    }
    fn library_list(
        &self,
        account: &str,
        kind: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<werc_domain::library::DocumentPage> {
        self.library_list_store(account, kind, before, limit)
    }
    fn library_delete(&self, account: &str, id: &str, revision: i64) -> AppResult<()> {
        self.library_delete_store(account, id, revision)
    }
    fn received_video_info(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<Option<(werc_domain::video::VideoAsset, String)>> {
        self.video_info_store(account, observation)
    }
    fn received_video(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<werc_domain::video::ProtectedVideo> {
        self.video_store(account, observation)
    }
    fn save_received_video(
        &self,
        data: &werc_domain::video::ProtectedVideo,
        binding: &werc_domain::video::VideoBinding<'_>,
    ) -> AppResult<()> {
        self.save_video_store(data, binding)
    }
    fn delete_received_video(&self, account: &str, observation: &str) -> AppResult<()> {
        self.delete_video_store(account, observation)
    }
    fn received_file_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.received_file_link_store(account, observation_id)
    }
    fn save_received_file(
        &self,
        file: &werc_domain::file::ProtectedFile,
        binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
    ) -> AppResult<()> {
        self.save_file_store_kind(file, Some(binding), true)
    }

    fn received_voice_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.received_voice_link_store(account, observation_id)
    }
    fn received_image_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.received_image_link_store(account, observation_id)
    }
    fn save_received_image_file(
        &self,
        file: &werc_domain::file::ProtectedFile,
        binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
    ) -> AppResult<()> {
        self.save_file_store_inner(file, Some(binding))
    }
    fn save_image(&self, asset: &werc_domain::image::ProtectedImage) -> AppResult<()> {
        self.save_image_store(asset)
    }
    fn images(&self, account: &str) -> AppResult<Vec<werc_domain::image::ImageAsset>> {
        self.images_store(account)
    }
    fn image(&self, account: &str, id: &str) -> AppResult<werc_domain::image::ProtectedImage> {
        self.image_store(account, id)
    }
    fn delete_image(&self, account: &str, id: &str) -> AppResult<()> {
        self.delete_image_store(account, id)
    }
    fn save_received_emoticon(
        &self,
        asset: &werc_domain::file::ProtectedFile,
        binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
        key_revision: i64,
    ) -> AppResult<()> {
        self.save_file_checked_store(
            asset,
            Some(binding),
            true,
            Some((binding.connection_revision, key_revision)),
        )
    }
    fn save_live_file(
        &self,
        asset: &werc_domain::file::ProtectedFile,
        revision: i64,
        key_revision: i64,
    ) -> AppResult<()> {
        self.save_file_checked_store(asset, None, false, Some((revision, key_revision)))
    }
    fn save_file(&self, asset: &werc_domain::file::ProtectedFile) -> AppResult<()> {
        self.save_file_store(asset)
    }
    fn files(&self, account: &str) -> AppResult<Vec<werc_domain::file::FileAsset>> {
        self.files_store(account)
    }
    fn file(&self, account: &str, id: &str) -> AppResult<werc_domain::file::ProtectedFile> {
        self.file_store(account, id)
    }
    fn delete_file(&self, account: &str, id: &str) -> AppResult<()> {
        self.delete_file_store(account, id)
    }
    fn commit_recovery(
        &self,
        data: &werc_domain::recovery::RecoveryCommit<'_>,
    ) -> AppResult<werc_domain::recovery::RecoverySummary> {
        self.commit_recovery_store(data)
    }
    fn recovery_summaries(
        &self,
        account: &str,
    ) -> AppResult<Vec<werc_domain::recovery::RecoverySummary>> {
        self.recovery_summaries_store(account)
    }
    fn recovery(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::recovery::ProtectedRecovery> {
        self.recovery_store(account, id)
    }
    fn bind_current_source(
        &self,
        account: &str,
        revision: i64,
        fingerprint: &str,
    ) -> AppResult<()> {
        self.bind_current_source_store(account, revision, fingerprint)
    }
    fn source_owner(&self, fingerprint: &str) -> AppResult<Option<String>> {
        self.connection()?
            .query_row(
                "SELECT account_id FROM setup_source_accounts WHERE source_fingerprint=?1",
                [fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())
    }
    fn native_identity(&self, account: &str) -> AppResult<Option<String>> {
        self.native_identity_store(account)
    }
    fn bind_native_identity(&self, account: &str, revision: i64, token: &str) -> AppResult<()> {
        self.bind_native_identity_store(account, revision, token)
    }
    fn receive_cursor(
        &self,
        account: &str,
    ) -> AppResult<Option<werc_domain::receive::ReceiveCursor>> {
        self.receive_cursor_store(account)
    }
    fn commit_receive(&self, data: &werc_domain::receive::ReceiveCommit<'_>) -> AppResult<bool> {
        self.commit_receive_store(data)
    }
    fn received_text_by_id(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Option<werc_domain::receive::ProtectedReceivedText>> {
        self.received_text_by_id_store(account, id)
    }
    fn received_texts(
        &self,
        account: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<(i64, werc_domain::receive::ProtectedReceivedText)>> {
        self.received_texts_store(account, after, limit)
    }
    fn projection(
        &self,
        account: &str,
        generation: &str,
    ) -> AppResult<Option<werc_domain::projection::ProtectedProjection>> {
        self.projection_store(account, generation)
    }
    fn begin_projection(
        &self,
        data: &werc_domain::projection::ProtectedProjection,
    ) -> AppResult<()> {
        self.begin_projection_store(data)
    }
    fn append_projection(
        &self,
        account: &str,
        id: &str,
        rows: &[werc_domain::projection::ProtectedProjectionRow],
    ) -> AppResult<()> {
        self.append_projection_store(account, id, rows)
    }
    fn finish_projection(
        &self,
        data: &werc_domain::projection::ProtectedProjection,
    ) -> AppResult<()> {
        self.finish_projection_store(data)
    }
    fn projection_row(
        &self,
        account: &str,
        id: &str,
        token: &str,
    ) -> AppResult<Option<werc_domain::projection::ProtectedProjectionRow>> {
        self.projection_row_store(account, id, token)
    }
    fn remove_projection(&self, account: &str, id: &str) -> AppResult<()> {
        self.remove_projection_store(account, id)
    }
    fn clear_projections(&self, account: &str) -> AppResult<usize> {
        self.clear_projections_store(account)
    }
    fn send_secret(&self, account: &str, candidate: Option<&[u8]>) -> AppResult<Option<Vec<u8>>> {
        self.send_secret_store(account, candidate)
    }
    fn next_queued_send(&self) -> AppResult<Option<werc_domain::outbox::SendStatus>> {
        self.next_queued_send_store()
    }
    fn pending_receipt_page(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<werc_domain::outbox::SendPage> {
        self.pending_receipt_page_store(account, before, limit)
    }
    fn create_send(
        &self,
        data: &werc_domain::outbox::ProtectedSend,
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.create_send_store(data)
    }
    fn send_status(&self, account: &str, id: &str) -> AppResult<werc_domain::outbox::SendStatus> {
        self.send_status_store(account, id)
    }
    fn send_payload(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::outbox::ProtectedSend> {
        self.send_payload_store(account, id)
    }
    fn sends(&self, account: &str) -> AppResult<Vec<werc_domain::outbox::SendStatus>> {
        self.sends_store(account)
    }
    fn send_by_idempotency(
        &self,
        account: &str,
        creator: &str,
        digest: &str,
    ) -> AppResult<Option<werc_domain::outbox::SendStatus>> {
        self.send_by_idempotency_store(account, creator, digest)
    }
    fn send_history_page(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<werc_domain::outbox::SendPage> {
        self.send_history_page_store(account, before, limit)
    }
    fn approve_send(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        until: i64,
        proof: &[u8],
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.approve_send_store(account, id, digest, context, until, proof)
    }
    fn approve_timed_send(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        timing: &werc_domain::outbox::SendTiming,
        proof: &[u8],
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.approve_timed_send_store(account, id, digest, context, timing, proof)
    }
    fn send_approval(&self, account: &str, id: &str) -> AppResult<Option<Vec<u8>>> {
        self.send_approval_store(account, id)
    }
    fn start_send(
        &self,
        expected: &werc_domain::outbox::SendStatus,
        attempt: &str,
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.start_send_store(expected, attempt)
    }
    fn record_send_receipt(
        &self,
        account: &str,
        receipt: &werc_domain::outbox::ProtectedReceipt,
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.record_send_receipt_store(account, receipt)
    }
    fn send_receipt(&self, account: &str, id: &str) -> AppResult<Option<Vec<u8>>> {
        self.send_receipt_store(account, id)
    }
    fn cancel_send(&self, account: &str, id: &str) -> AppResult<werc_domain::outbox::SendStatus> {
        self.cancel_send_store(account, id)
    }
    fn mark_send_unknown(
        &self,
        account: &str,
        id: &str,
        attempt: &str,
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        self.mark_send_unknown_store(account, id, attempt)
    }
    fn recover_sends(&self) -> AppResult<usize> {
        self.recover_sends_store()
    }

    fn current_connection(&self) -> AppResult<werc_domain::current::CurrentConnection> {
        self.current_connection_store()
    }
    fn select_current_connection(&self, account: &str, revision: &str) -> AppResult<()> {
        self.select_current_store(account, revision)
    }
    fn setup_payload(&self) -> AppResult<Option<Vec<u8>>> {
        self.connection()?
            .query_row(
                "SELECT payload FROM setup_state WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())
    }
    fn save_setup_payload(&self, payload: Option<&[u8]>) -> AppResult<()> {
        // ProtectedSetup serializes ciphertext as a JSON byte array (up to 4x).
        if payload.is_some_and(|v| v.len() > 32 * 1024 * 1024) {
            return Err(AppError::new("QUOTA_EXCEEDED", "引导状态超限"));
        }
        self.connection()?
            .execute(
                "UPDATE setup_state SET payload=?1 WHERE singleton=1",
                [payload],
            )
            .map_err(|_| AppError::internal())?;
        Ok(())
    }
    fn setup_account(&self, label: &str) -> AppResult<Account> {
        self.setup_account_store(label)
    }
    fn setup_account_for_source(&self, label: &str, fingerprint: &str) -> AppResult<Account> {
        self.setup_account_for_source_store(label, fingerprint)
    }
    fn setup_dismissed(&self) -> AppResult<bool> {
        self.connection()?
            .query_row(
                "SELECT dismissed FROM setup_state WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())
    }
    fn dismiss_setup(&self) -> AppResult<()> {
        self.connection()?
            .execute("UPDATE setup_state SET dismissed=1 WHERE singleton=1", [])
            .map_err(|_| AppError::internal())?;
        Ok(())
    }
    fn history_sources(
        &self,
        account: &str,
    ) -> AppResult<Vec<werc_domain::history::HistorySource>> {
        self.history_sources_store(account)
    }
    fn history_payload(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::history::ProtectedHistory> {
        self.history_payload_store(account, id)
    }
    fn save_history(&self, data: &werc_domain::history::ProtectedHistory) -> AppResult<()> {
        self.save_history_store(data)
    }
    fn begin_history(&self, data: &werc_domain::history::ProtectedHistory) -> AppResult<()> {
        self.begin_history_store(data)
    }
    fn append_history_batch(
        &self,
        account: &str,
        id: &str,
        batch: &werc_domain::history::ProtectedHistoryBatch,
    ) -> AppResult<()> {
        self.append_history_batch_store(account, id, batch)
    }
    fn finish_history(
        &self,
        data: &werc_domain::history::ProtectedHistory,
        expected_revision: i64,
    ) -> AppResult<()> {
        self.finish_history_store(data, expected_revision)
    }
    fn abort_history(&self, account: &str, id: &str) -> AppResult<()> {
        self.abort_history_store(account, id)
    }
    fn history_batch(
        &self,
        account: &str,
        id: &str,
        sequence: u64,
    ) -> AppResult<Option<werc_domain::history::ProtectedHistoryBatch>> {
        self.history_batch_store(account, id, sequence)
    }
    fn history_message_states(
        &self,
        account: &str,
        ids: &[String],
    ) -> AppResult<std::collections::BTreeMap<String, werc_domain::history::HistoryMessageState>>
    {
        self.history_message_states_store(account, ids)
    }
    fn history_message_digest(&self, account: &str, message_id: &str) -> AppResult<Option<String>> {
        self.history_message_digest_store(account, message_id)
    }
    fn history_message_is_current(
        &self,
        account: &str,
        source: &str,
        message_id: &str,
    ) -> AppResult<bool> {
        self.history_message_is_current_store(account, source, message_id)
    }
    fn history_index_digest(&self, account: &str, source: &str) -> AppResult<String> {
        self.history_index_digest_store(account, source, &|| Ok(()))
    }
    fn history_index_digest_controlled(
        &self,
        account: &str,
        source: &str,
        check: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<String> {
        self.history_index_digest_store(account, source, check)
    }
    fn history_index_key(
        &self,
        account: &str,
        candidate: Option<&[u8]>,
    ) -> AppResult<Option<Vec<u8>>> {
        let c = self.connection()?;
        if let Some(candidate) = candidate {
            if candidate.len() > 65536 {
                return Err(AppError::new("INVALID_ARGUMENT", "受保护索引密钥超限"));
            }
            c.execute("INSERT INTO history_index_keys(account_id,wrapped_key) VALUES(?1,?2) ON CONFLICT(account_id) DO NOTHING",params![account,candidate]).map_err(|_| AppError::internal())?;
        }
        c.query_row(
            "SELECT wrapped_key FROM history_index_keys WHERE account_id=?1",
            [account],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::internal())
    }
    fn remove_history(&self, account: &str, id: &str) -> AppResult<()> {
        self.remove_history_store(account, id)
    }

    fn queue_operation(
        &self,
        operation: &werc_domain::operation::Operation,
        key: &str,
    ) -> AppResult<werc_domain::operation::Operation> {
        self.queue_operation_store(operation, key)
    }
    fn operations(&self, account: &str) -> AppResult<Vec<werc_domain::operation::Operation>> {
        self.operations_store(account)
    }
    fn operation(&self, account: &str, id: &str) -> AppResult<werc_domain::operation::Operation> {
        self.operation_store(account, id)
    }
    fn claim_operation(&self) -> AppResult<Option<werc_domain::operation::Operation>> {
        self.claim_operation_store()
    }
    fn finish_operation(&self, operation: &werc_domain::operation::Operation) -> AppResult<()> {
        self.finish_operation_store(operation)
    }
    fn cancel_operation(&self, account: &str, id: &str) -> AppResult<()> {
        self.cancel_operation_store(account, id)
    }
    fn recover_operations(&self) -> AppResult<usize> {
        self.recover_operations_store()
    }
    fn save_asset(&self, asset: &werc_domain::media::ProtectedAsset) -> AppResult<()> {
        self.save_asset_store(asset)
    }
    fn save_live_audio(
        &self,
        asset: &werc_domain::media::ProtectedAsset,
        revision: i64,
        key_revision: i64,
    ) -> AppResult<()> {
        self.save_asset_checked_store(asset, Some((revision, key_revision)))
    }
    fn assets(&self, account: &str) -> AppResult<Vec<werc_domain::media::AudioAsset>> {
        self.assets_store(account)
    }
    fn asset(&self, account: &str, id: &str) -> AppResult<werc_domain::media::ProtectedAsset> {
        self.asset_store(account, id)
    }
    fn delete_asset(&self, account: &str, id: &str) -> AppResult<()> {
        self.delete_asset_store(account, id)
    }
    fn bindings(&self) -> AppResult<Vec<werc_domain::connection::ClientBinding>> {
        self.bindings_store()
    }
    fn save_binding(&self, binding: &werc_domain::connection::ClientBinding) -> AppResult<()> {
        self.save_binding_store(binding)
    }
    fn remove_binding(&self, account: &str) -> AppResult<()> {
        self.remove_binding_store(account)
    }
    fn store_key(
        &self,
        binding: &werc_domain::connection::ClientBinding,
        protected: &[u8],
    ) -> AppResult<()> {
        self.store_key_store(binding, protected, None)
    }
    fn key_revision(&self, account: &str) -> AppResult<i64> {
        self.key_revision_store(account)
    }
    fn store_acquired_key(
        &self,
        binding: &werc_domain::connection::ClientBinding,
        protected: &[u8],
        expected_revision: i64,
    ) -> AppResult<()> {
        self.store_key_store(binding, protected, Some(expected_revision))
    }
    fn protected_key(&self, account: &str) -> AppResult<Option<(Vec<u8>, String)>> {
        self.protected_key_store(account)
    }
    fn remove_key(&self, account: &str) -> AppResult<()> {
        self.remove_key_store(account)
    }
    fn accounts(&self) -> AppResult<Vec<Account>> {
        let c = self.connection()?;
        let mut statement = c
            .prepare("SELECT id,label,created_at FROM accounts ORDER BY created_at,id")
            .map_err(|_| AppError::internal())?;
        statement
            .query_map([], |r| {
                Ok(Account {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    created_at: r.get(2)?,
                    state: AccountState::Registered,
                })
            })
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())
    }

    fn register_account(&self, label: &str) -> AppResult<Account> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let account = Self::claim_current(&tx, label, false)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(account)
    }

    fn has_pending_tasks(&self, account: &str) -> AppResult<bool> {
        self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE account_id=?1 AND state IN ('queued','preparing','awaiting_user','submitting')) OR EXISTS(SELECT 1 FROM send_outbox WHERE account_id=?1 AND state IN ('queued','submitting'))",
            [account], |row| row.get(0),
        ).map_err(|_| AppError::internal())
    }

    fn remove_account(&self, id: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE account_id=?1 AND state IN ('queued','preparing','awaiting_user','submitting')) OR EXISTS(SELECT 1 FROM send_outbox WHERE account_id=?1 AND state IN ('queued','submitting'))",
            [id], |row| row.get(0),
        ).map_err(|_| AppError::internal())?;
        if pending {
            return Err(AppError::new("ACCOUNT_BUSY", "此配置仍有待处理任务，请先完成或取消后再移除"));
        }
        tx.execute("UPDATE current_connection SET account_id=NULL,revision=revision+1,selection_required=EXISTS(SELECT 1 FROM accounts WHERE id<>?1) WHERE account_id=?1", [id]).map_err(|_| AppError::internal())?;
        tx.execute("UPDATE tokens SET revoked=1 WHERE id IN (SELECT token_id FROM token_accounts WHERE account_id=?1)", [id]).map_err(|_| AppError::internal())?;
        if tx
            .execute("DELETE FROM accounts WHERE id=?1", [id])
            .map_err(|_| AppError::internal())?
            == 0
        {
            return Err(AppError::new("ACCOUNT_NOT_FOUND", "登记账号不存在"));
        }
        tx.execute("DELETE FROM events WHERE account_id=?1", [id])
            .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }

    fn create_token(&self, info: &TokenInfo, digest: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        if info.account_ids.len() != 1 {
            return Err(AppError::new(
                "CURRENT_CONNECTION_REQUIRED",
                "令牌必须限定唯一当前微信",
            ));
        }
        current_store::require_current(&tx, &info.account_ids[0])?;
        let json = serde_json::to_string(info).map_err(|_| AppError::internal())?;
        tx.execute(
            "INSERT INTO tokens(id,digest,info,expires_at) VALUES (?1,?2,?3,?4)",
            params![info.id, digest, json, info.expires_at],
        )
        .map_err(|_| AppError::internal())?;
        for account in &info.account_ids {
            tx.execute(
                "INSERT INTO token_accounts(token_id,account_id) VALUES (?1,?2)",
                params![info.id, account],
            )
            .map_err(|_| AppError::internal())?;
        }
        tx.commit().map_err(|_| AppError::internal())
    }

    fn tokens(&self) -> AppResult<Vec<TokenInfo>> {
        let c = self.connection()?;
        let mut statement = c
            .prepare("SELECT info,revoked FROM tokens ORDER BY rowid DESC")
            .map_err(|_| AppError::internal())?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))
            .map_err(|_| AppError::internal())?;
        rows.map(|row| {
            let (json, revoked) = row.map_err(|_| AppError::internal())?;
            let mut info: TokenInfo =
                serde_json::from_str(&json).map_err(|_| AppError::internal())?;
            info.revoked = revoked;
            Ok(info)
        })
        .collect()
    }

    fn revoke_token(&self, id: &str) -> AppResult<()> {
        if self
            .connection()?
            .execute("UPDATE tokens SET revoked=1 WHERE id=?1", [id])
            .map_err(|_| AppError::internal())?
            == 0
        {
            return Err(AppError::new("TOKEN_NOT_FOUND", "令牌不存在"));
        }
        Ok(())
    }

    fn authenticate(&self, digest: &str) -> AppResult<Principal> {
        let c = self.connection()?;
        let json: Option<String> = c
            .query_row(
                "SELECT info FROM tokens WHERE digest=?1 AND revoked=0 AND (expires_at>?2 OR expires_at=?3) AND (SELECT COUNT(*) FROM token_accounts a WHERE a.token_id=tokens.id)=1 AND EXISTS(SELECT 1 FROM token_accounts a JOIN current_connection c ON a.account_id=c.account_id WHERE a.token_id=tokens.id)",
                params![digest, Utc::now().to_rfc3339(), werc_domain::PERMANENT_TOKEN_EXPIRY],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let info: TokenInfo = serde_json::from_str(
            &json.ok_or_else(|| AppError::new("UNAUTHORIZED", "令牌无效、过期或已撤销"))?,
        )
        .map_err(|_| AppError::internal())?;
        let mut statement = c
            .prepare("SELECT account_id FROM token_accounts WHERE token_id=?1")
            .map_err(|_| AppError::internal())?;
        let account_ids = statement
            .query_map([&info.id], |r| r.get(0))
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|_| AppError::internal())?;
        Ok(Principal {
            id: info.id,
            account_ids,
            scopes: info.scopes,
        })
    }

    fn events(&self, after: i64, account: &str, limit: u32) -> AppResult<Vec<Event>> {
        let c = self.connection()?;
        let mut statement = c.prepare("SELECT seq,event_type,occurred_at,payload FROM events WHERE seq>?1 AND account_id=?2 ORDER BY seq LIMIT ?3").map_err(|_| AppError::internal())?;
        let rows = statement
            .query_map(params![after, account, limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| AppError::internal())?;
        rows.map(|row| {
            let (event_id, event_type, occurred_at, payload) =
                row.map_err(|_| AppError::internal())?;
            Ok(Event {
                event_id,
                schema_version: 1,
                event_type,
                account_id: Some(account.into()),
                observed_at: occurred_at.clone(),
                occurred_at,
                payload: serde_json::from_str(&payload).map_err(|_| AppError::internal())?,
            })
        })
        .collect()
    }
}
