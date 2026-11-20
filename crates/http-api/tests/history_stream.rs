#![cfg(windows)]

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, capability::Capability, history::*};
use werc_infrastructure::SqliteRepository;

const START_TIME: i64 = 1_700_000_000;
const SOURCE_DIGEST: &str = "abababababababababababababababababababababababababababababababab";
const CONVERSATION: &str = "synthetic-stream-conversation";
const EXACT_WIRE_LIMIT_CASE: u8 = 1;
const LATE_OLD_SESSION_CASE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Fault {
    None,
    Schema,
    ErrorEvent,
    ReaderFailure,
    Digest,
    SequenceGap,
    EndCount,
    MissingEnd,
    ErrorAfterEnd,
    TooManyRows,
    TooManyBytes,
}
impl Fault {
    fn code(self) -> &'static str {
        match self {
            Self::Schema => "SCHEMA_UNSUPPORTED",
            Self::ErrorEvent | Self::ReaderFailure | Self::ErrorAfterEnd => "READER_FAILED",
            _ => "READER_PROTOCOL_INVALID",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    AfterBegin,
    AfterFirstBatch,
    BeforeEnd,
    AfterEnd,
}
type Observer = Box<dyn Fn(Stage) + Send + Sync>;

#[derive(Clone, Copy, Debug, Default)]
struct StreamStats {
    batches: usize,
    rows: usize,
    serialized_bytes: usize,
    max_batch_rows: usize,
    max_batch_bytes: usize,
}

// Generates and moves only one batch at a time; never constructs a 60k-row
// HistoryBundle or retains previously delivered message text in the provider.
struct StreamingProvider {
    rows: usize,
    batch_rows: usize,
    text_bytes: usize,
    fault: AtomicU8,
    regression_case: AtomicU8,
    observer: Mutex<Option<Observer>>,
    stats: Mutex<StreamStats>,
}
impl StreamingProvider {
    fn new(rows: usize, batch_rows: usize, text_bytes: usize) -> Self {
        assert!(batch_rows > 0 && batch_rows <= HISTORY_BATCH_ROWS);
        Self {
            rows,
            batch_rows,
            text_bytes,
            fault: AtomicU8::new(Fault::None as u8),
            regression_case: AtomicU8::new(0),
            observer: Mutex::new(None),
            stats: Mutex::new(StreamStats::default()),
        }
    }
    fn stream_regression_case(
        &self,
        case: u8,
        visitor: &mut dyn FnMut(HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        let template = HistoryBundle {
            schema: "synthetic-stream/v2".into(),
            source_sha256: SOURCE_DIGEST.into(),
            encryption: "synthetic-plaintext-input".into(),
            wal_committed_frames: 0,
            messages: Vec::new(),
            contacts: Vec::new(),
            sessions: Vec::new(),
            warnings: Vec::new(),
        };
        let batches = match case {
            EXACT_WIRE_LIMIT_CASE => {
                let mut data = template.clone();
                let mut message = self.message(0);
                message.id = "x".into();
                message.text = Some(String::new());
                data.messages.push(message);
                // Plain ASCII padding increases JSON size by exactly one byte
                // per character; measure the real encoding rather than guess.
                let overhead = serde_json::to_vec(&data).unwrap().len();
                data.messages[0].text = Some("x".repeat(HISTORY_BATCH_BYTES - overhead));
                assert_eq!(
                    serde_json::to_vec(&data).unwrap().len(),
                    HISTORY_BATCH_BYTES
                );
                vec![data]
            }
            LATE_OLD_SESSION_CASE => {
                let mut messages = template.clone();
                let mut message = self.message(0);
                message.conversation_id = "C".into();
                message.timestamp = 200;
                message.text = Some("new".into());
                messages.messages.push(message);
                let mut sessions = template.clone();
                sessions.sessions.push(HistorySession {
                    username: "C".into(),
                    summary: Some("old".into()),
                    timestamp: 100,
                    unread_count: Some(7),
                });
                vec![messages, sessions]
            }
            _ => panic!("unknown regression case"),
        };
        visitor(HistoryReadEvent::Begin {
            schema: template.schema,
            source_sha256: template.source_sha256,
            encryption: template.encryption,
            wal_committed_frames: 0,
        })?;
        let batch_count = batches.len();
        let mut counts = (0usize, 0usize, 0usize);
        *self.stats.lock().unwrap() = StreamStats::default();
        for (sequence, data) in batches.into_iter().enumerate() {
            let bytes = serde_json::to_vec(&data).unwrap();
            let digest = hex::encode(Sha256::digest(&bytes));
            counts.0 += data.messages.len();
            counts.1 += data.contacts.len();
            counts.2 += data.sessions.len();
            {
                let mut stats = self.stats.lock().unwrap();
                stats.batches += 1;
                stats.rows += data.messages.len() + data.contacts.len() + data.sessions.len();
                stats.serialized_bytes += bytes.len();
                stats.max_batch_rows = stats
                    .max_batch_rows
                    .max(data.messages.len() + data.contacts.len() + data.sessions.len());
                stats.max_batch_bytes = stats.max_batch_bytes.max(bytes.len());
            }
            drop(bytes);
            visitor(HistoryReadEvent::Batch {
                sequence: sequence as u64,
                digest,
                data,
            })?;
        }
        visitor(HistoryReadEvent::End {
            batches: batch_count as u64,
            message_count: counts.0,
            contact_count: counts.1,
            session_count: counts.2,
        })
    }
    fn set_fault(&self, fault: Fault) {
        self.fault.store(fault as u8, Ordering::SeqCst);
    }
    fn is_fault(&self, fault: Fault) -> bool {
        self.fault.load(Ordering::SeqCst) == fault as u8
    }
    fn observe(&self, stage: Stage) {
        if let Some(observer) = self.observer.lock().unwrap().as_ref() {
            observer(stage);
        }
    }
    fn message(&self, index: usize) -> HistoryMessage {
        let prefix = format!("stream-secret-{index:06}-");
        let text = format!(
            "{prefix}{}",
            "x".repeat(self.text_bytes.saturating_sub(prefix.len()))
        );
        HistoryMessage {
            id: "reader-id-must-be-normalized".into(),
            conversation_id: CONVERSATION.into(),
            sender_id: Some("synthetic-sender".into()),
            local_id: (index + 1) as i64,
            server_id: (index + 1).to_string(),
            // Ties exercise the stable (timestamp,sequence,id) cursor ordering.
            timestamp: START_TIME + (index / 3) as i64,
            sequence: index as i64,
            message_type: 1,
            text: Some(text),
            content_status: "decoded".into(),
        }
    }
}
impl CapabilityProvider for StreamingProvider {
    fn capabilities(&self) -> Vec<Capability> {
        Vec::new()
    }
    fn protect_key(&self, context: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        werc_wechat_win::keys::protect(context, key)
    }
    fn unprotect_key(&self, context: &str, protected: &[u8]) -> AppResult<Vec<u8>> {
        werc_wechat_win::keys::unprotect(context, protected)
    }
    fn read_database(&self, _: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        panic!("stream import must not fall back to materializing a whole source")
    }
    fn stream_database(
        &self,
        request: &HistoryReadRequest,
        visitor: &mut dyn FnMut(HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        assert!(
            request.allow_plaintext,
            "synthetic application-layer stream fixture"
        );
        let case = self.regression_case.load(Ordering::SeqCst);
        if case != 0 {
            return self.stream_regression_case(case, visitor);
        }
        *self.stats.lock().unwrap() = StreamStats::default();
        visitor(HistoryReadEvent::Begin {
            schema: "synthetic-stream/v2".into(),
            source_sha256: SOURCE_DIGEST.into(),
            encryption: "synthetic-plaintext-input".into(),
            wal_committed_frames: 0,
        })?;
        self.observe(Stage::AfterBegin);
        let mut batches = 0usize;
        for start in (0..self.rows).step_by(self.batch_rows) {
            let end = (start + self.batch_rows).min(self.rows);
            let mut data = HistoryBundle {
                schema: "synthetic-stream/v2".into(),
                source_sha256: SOURCE_DIGEST.into(),
                encryption: "synthetic-plaintext-input".into(),
                wal_committed_frames: 0,
                // Reverse each batch so pagination cannot rely on input order.
                messages: (start..end).rev().map(|i| self.message(i)).collect(),
                contacts: Vec::new(),
                sessions: Vec::new(),
                warnings: Vec::new(),
            };
            if batches == 1 && self.is_fault(Fault::TooManyRows) {
                while data.messages.len() <= HISTORY_BATCH_ROWS {
                    data.messages
                        .push(self.message(self.rows + data.messages.len()));
                }
            }
            if batches == 1 && self.is_fault(Fault::TooManyBytes) {
                data.messages[0].text = Some("x".repeat(HISTORY_BATCH_BYTES + 1));
            }
            let bytes = serde_json::to_vec(&data).unwrap();
            let mut digest = hex::encode(Sha256::digest(&bytes));
            {
                let mut stats = self.stats.lock().unwrap();
                stats.batches += 1;
                stats.rows += data.messages.len();
                stats.serialized_bytes += bytes.len();
                stats.max_batch_rows = stats.max_batch_rows.max(data.messages.len());
                stats.max_batch_bytes = stats.max_batch_bytes.max(bytes.len());
            }
            drop(bytes);
            if batches == 1 && self.is_fault(Fault::Digest) {
                digest = "0".repeat(64);
            }
            let sequence = if batches == 1 && self.is_fault(Fault::SequenceGap) {
                batches as u64 + 1
            } else {
                batches as u64
            };
            visitor(HistoryReadEvent::Batch {
                sequence,
                digest,
                data,
            })?;
            if batches == 0 {
                self.observe(Stage::AfterFirstBatch);
            }
            batches += 1;
            if batches == 1 {
                if self.is_fault(Fault::Schema) {
                    return visitor(HistoryReadEvent::Error {
                        code: "SCHEMA_UNSUPPORTED".into(),
                    });
                }
                if self.is_fault(Fault::ErrorEvent) {
                    return visitor(HistoryReadEvent::Error {
                        code: "READER_FAILED".into(),
                    });
                }
                if self.is_fault(Fault::ReaderFailure) {
                    return Err(AppError::new("READER_FAILED", "synthetic stream failure"));
                }
            }
        }
        if self.is_fault(Fault::MissingEnd) {
            return Ok(());
        }
        self.observe(Stage::BeforeEnd);
        visitor(HistoryReadEvent::End {
            batches: batches as u64,
            message_count: self.rows + usize::from(self.is_fault(Fault::EndCount)),
            contact_count: 0,
            session_count: 0,
        })?;
        self.observe(Stage::AfterEnd);
        if self.is_fault(Fault::ErrorAfterEnd) {
            return Err(AppError::new(
                "READER_FAILED",
                "synthetic transport failure after end",
            ));
        }
        Ok(())
    }
}

struct TempDirectory(PathBuf);
impl TempDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("werc-history-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Fixture {
    // Keep the directory last: application, repository, and provider are dropped
    // before removing the Windows SQLite file.
    app: Arc<Application>,
    repository: Arc<SqliteRepository>,
    provider: Arc<StreamingProvider>,
    account: String,
    directory: TempDirectory,
}
impl Fixture {
    fn new(rows: usize, batch_rows: usize, text_bytes: usize) -> Self {
        let directory = TempDirectory::new();
        let repository =
            Arc::new(SqliteRepository::open(&directory.0.join("state.sqlite3")).unwrap());
        let provider = Arc::new(StreamingProvider::new(rows, batch_rows, text_bytes));
        let app = Arc::new(Application::new(repository.clone(), provider.clone()));
        let account = app
            .register_account_local("synthetic stream account")
            .unwrap()
            .id;
        Self {
            app,
            repository,
            provider,
            account,
            directory,
        }
    }
    fn request(&self) -> HistoryReadRequest {
        HistoryReadRequest {
            path: self
                .directory
                .0
                .join("synthetic-source.db")
                .to_str()
                .unwrap()
                .into(),
            key_hex: None,
            allow_plaintext: true,
        }
    }
    fn import(&self) -> AppResult<HistorySource> {
        self.app
            .import_history_local(&self.account, self.request(), true)
    }
    fn connection(&self) -> Connection {
        Connection::open(self.directory.0.join("state.sqlite3")).unwrap()
    }
    fn assert_no_committed_or_staged_history(&self) {
        assert!(
            self.repository
                .history_sources(&self.account)
                .unwrap()
                .is_empty()
        );
        assert!(
            self.repository
                .events(0, &self.account, 200)
                .unwrap()
                .iter()
                .all(|e| e.event_type != "history.imported"
                    && e.event_type != "history.message_observed")
        );
        let c = self.connection();
        for table in [
            "history_sources",
            "history_batches",
            "history_record_digests",
            "history_pending_events",
        ] {
            let count: i64 = c
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "failed stream leaked {table}");
        }
    }
}
fn query(limit: u32) -> HistoryQuery {
    HistoryQuery {
        conversation_id: None,
        sender_id: None,
        message_type: None,
        search: None,
        from: None,
        until: None,
        cursor: None,
        limit,
    }
}
fn assert_source_corrupted(f: &Fixture) {
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(2))
            .unwrap_err()
            .code,
        "SOURCE_CORRUPTED"
    );
}

#[test]
fn sixty_thousand_messages_stream_in_120_bounded_batches_and_page_stably_past_32_mib() {
    const ROWS: usize = 60_000;
    let f = Fixture::new(ROWS, 500, 768);
    let source = f.import().unwrap();
    assert_eq!(source.message_count, ROWS);
    assert_eq!(source.contact_count, 0);
    assert_eq!(source.session_count, 0);
    assert!(!source.real_wechat_verified);
    let stats = *f.provider.stats.lock().unwrap();
    assert_eq!(stats.batches, 120);
    assert_eq!(stats.rows, ROWS);
    assert_eq!(stats.max_batch_rows, 500);
    assert!(stats.serialized_bytes > 32 * 1024 * 1024);
    assert!(stats.max_batch_bytes <= HISTORY_BATCH_BYTES);
    let c = f.connection();
    let (batches, stored_bytes, stored_records): (i64, i64, i64) = c.query_row(
        "SELECT batch_count,payload_bytes,(SELECT count(*) FROM history_record_digests d WHERE d.source_id=s.id) FROM history_sources s WHERE id=?1 AND visible=1",
        [&source.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(batches, 120);
    assert_eq!(stored_records, ROWS as i64);
    assert!(stored_bytes > 32 * 1024 * 1024);
    let committed_changes: i64 = c.query_row(
        "SELECT count(*) FROM events WHERE account_id=?1 AND event_type='history.message_observed'",
        [&f.account], |r| r.get(0),
    ).unwrap();
    assert_eq!(committed_changes, ROWS as i64);

    let manifest = f
        .repository
        .history_payload(&f.account, &source.id)
        .unwrap();
    assert_eq!(manifest.version, 2);
    let mut nonces = BTreeSet::new();
    assert!(nonces.insert(manifest.nonce));
    for sequence in 0..120 {
        let batch = f
            .repository
            .history_batch(&f.account, &source.id, sequence)
            .unwrap()
            .unwrap();
        assert_eq!(batch.nonce.len(), 12);
        assert!(
            nonces.insert(batch.nonce),
            "AES-GCM nonce reuse in one source"
        );
        assert!(batch.ciphertext.len() <= HISTORY_BATCH_BYTES + 16);
        assert!(
            !batch
                .ciphertext
                .windows(b"stream-secret-".len())
                .any(|w| w == b"stream-secret-")
        );
    }
    assert!(
        f.repository
            .history_batch(&f.account, &source.id, 120)
            .unwrap()
            .is_none()
    );

    // Query only the first page, its stable repeat, and two tail-filtered pages;
    // do not repeatedly rescan all 60k records for hundreds of cursor pages.
    let first = f.app.history_messages_local(&f.account, &query(2)).unwrap();
    assert_eq!(
        first.data.iter().map(|m| m.local_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(first.next_cursor.is_some());
    assert_eq!(first.source_ids, vec![source.id.clone()]);
    let repeated = f.app.history_messages_local(&f.account, &query(2)).unwrap();
    assert_eq!(first.next_cursor, repeated.next_cursor);
    assert_eq!(first.data[0].id, repeated.data[0].id);
    let mut tail = query(4);
    tail.conversation_id = Some(CONVERSATION.into());
    tail.from = Some(START_TIME + ((ROWS - 6) / 3) as i64);
    let before_last = f.app.history_messages_local(&f.account, &tail).unwrap();
    assert_eq!(
        before_last
            .data
            .iter()
            .map(|m| m.local_id)
            .collect::<Vec<_>>(),
        vec![59_995, 59_996, 59_997, 59_998]
    );
    assert!(before_last.next_cursor.is_some());
    tail.cursor = before_last.next_cursor;
    let last = f.app.history_messages_local(&f.account, &tail).unwrap();
    assert_eq!(
        last.data.iter().map(|m| m.local_id).collect::<Vec<_>>(),
        vec![59_999, 60_000]
    );
    assert!(last.next_cursor.is_none());
    assert_eq!(last.data_mode, "imported_snapshot");
    assert_eq!(f.repository.history_sources(&f.account).unwrap().len(), 1);
}

#[test]
fn begin_and_batches_and_end_are_invisible_until_the_stream_returns_success() {
    let f = Fixture::new(6, 2, 768);
    let weak = Arc::downgrade(&f.app);
    let repository = f.repository.clone();
    let account = f.account.clone();
    let database_path = f.directory.0.join("state.sqlite3");
    let observed = Arc::new(Mutex::new(Vec::new()));
    let stages = observed.clone();
    *f.provider.observer.lock().unwrap() = Some(Box::new(move |stage| {
        stages.lock().unwrap().push(stage);
        let c = Connection::open(&database_path).unwrap();
        let (pending, visible): (i64, i64) = c
            .query_row(
                "SELECT count(*),coalesce(sum(visible),0) FROM history_sources",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((pending, visible), (1, 0));
        let staged_batches: i64 = c
            .query_row("SELECT count(*) FROM history_batches", [], |r| r.get(0))
            .unwrap();
        let expected_batches = match stage {
            Stage::AfterBegin => 0,
            Stage::AfterFirstBatch => 1,
            Stage::BeforeEnd | Stage::AfterEnd => 3,
        };
        assert_eq!(staged_batches, expected_batches);
        let app = weak.upgrade().unwrap();
        assert!(app.history_sources_local(&account).unwrap().is_empty());
        assert_eq!(
            app.history_messages_local(&account, &query(1))
                .unwrap_err()
                .code,
            "CAPABILITY_NOT_IMPLEMENTED"
        );
        assert!(
            repository
                .events(0, &account, 200)
                .unwrap()
                .iter()
                .all(|e| e.event_type != "history.imported"
                    && e.event_type != "history.message_observed")
        );
        let normalized_id = hex::encode(Sha256::digest(
            format!("{CONVERSATION}\nserver:1").as_bytes(),
        ));
        assert!(
            repository
                .history_message_digest(&account, &normalized_id)
                .unwrap()
                .is_none()
        );
    }));
    let source = f.import().unwrap();
    assert_eq!(
        *observed.lock().unwrap(),
        vec![
            Stage::AfterBegin,
            Stage::AfterFirstBatch,
            Stage::BeforeEnd,
            Stage::AfterEnd
        ]
    );
    assert_eq!(f.repository.history_sources(&f.account).unwrap().len(), 1);
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(10))
            .unwrap()
            .data
            .len(),
        6
    );
    assert_eq!(source.message_count, 6);
}

#[test]
fn schema_errors_protocol_corruption_and_missing_end_abort_every_staged_row_and_allow_retry() {
    for fault in [
        Fault::Schema,
        Fault::ErrorEvent,
        Fault::ReaderFailure,
        Fault::Digest,
        Fault::SequenceGap,
        Fault::EndCount,
        Fault::MissingEnd,
        Fault::ErrorAfterEnd,
        Fault::TooManyRows,
        Fault::TooManyBytes,
    ] {
        let f = Fixture::new(6, 2, 768);
        f.provider.set_fault(fault);
        assert_eq!(f.import().unwrap_err().code, fault.code(), "{fault:?}");
        f.assert_no_committed_or_staged_history();
        f.provider.set_fault(Fault::None);
        let source = f.import().unwrap();
        assert_eq!(source.message_count, 6, "{fault:?} retry");
        assert_eq!(f.repository.history_sources(&f.account).unwrap().len(), 1);
        assert_eq!(
            f.app
                .history_messages_local(&f.account, &query(10))
                .unwrap()
                .data
                .len(),
            6
        );
        let duplicate = f.import().unwrap();
        assert_eq!(duplicate.id, source.id);
        let c = f.connection();
        assert_eq!(
            c.query_row("SELECT count(*) FROM history_sources", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM history_pending_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn a_changed_key_revision_rejects_late_end_and_post_end_commit_then_allows_reconfirmation() {
    for change_at in [Stage::BeforeEnd, Stage::AfterEnd] {
        let f = Fixture::new(6, 2, 768);
        let repository = f.repository.clone();
        let account = f.account.clone();
        let revision = repository.key_revision(&account).unwrap();
        *f.provider.observer.lock().unwrap() = Some(Box::new(move |stage| {
            if stage == change_at {
                repository.remove_key(&account).unwrap();
            }
        }));
        assert_eq!(f.import().unwrap_err().code, "CONNECTION_CHANGED");
        assert_ne!(f.repository.key_revision(&f.account).unwrap(), revision);
        f.assert_no_committed_or_staged_history();
        *f.provider.observer.lock().unwrap() = None;
        assert_eq!(f.import().unwrap().message_count, 6);
    }
}

#[test]
fn manifest_cipher_nonce_and_metadata_corruption_fail_closed_on_query() {
    let f = Fixture::new(8, 2, 768);
    let source = f.import().unwrap();
    let c = f.connection();
    let (original_ciphertext, original_nonce, original_metadata): (Vec<u8>, Vec<u8>, String) = c
        .query_row(
            "SELECT ciphertext,nonce,metadata FROM history_sources WHERE id=?1",
            [&source.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let mut corrupted = original_ciphertext.clone();
    corrupted[0] ^= 0x80;
    c.execute(
        "UPDATE history_sources SET ciphertext=?1 WHERE id=?2",
        params![corrupted, source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_sources SET ciphertext=?1 WHERE id=?2",
        params![original_ciphertext, source.id],
    )
    .unwrap();
    let mut bad_nonce = original_nonce.clone();
    bad_nonce[0] ^= 1;
    c.execute(
        "UPDATE history_sources SET nonce=?1 WHERE id=?2",
        params![bad_nonce, source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_sources SET nonce=?1 WHERE id=?2",
        params![original_nonce, source.id],
    )
    .unwrap();
    let mut metadata: serde_json::Value = serde_json::from_str(&original_metadata).unwrap();
    metadata["message_count"] = serde_json::json!(9);
    c.execute(
        "UPDATE history_sources SET metadata=?1 WHERE id=?2",
        params![metadata.to_string(), source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_sources SET metadata=?1 WHERE id=?2",
        params![original_metadata, source.id],
    )
    .unwrap();
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(10))
            .unwrap()
            .data
            .len(),
        8
    );
}

#[test]
fn authenticated_record_index_tampering_or_missing_account_index_key_cannot_silently_hide_rows() {
    let f = Fixture::new(8, 2, 768);
    let source = f.import().unwrap();
    let c = f.connection();
    let (message_id, digest): (String, String) = c.query_row(
        "SELECT message_id,digest FROM history_record_digests WHERE source_id=?1 ORDER BY message_id LIMIT 1",
        [&source.id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    let tampered_digest = if digest == "0".repeat(64) {
        "1".repeat(64)
    } else {
        "0".repeat(64)
    };
    c.execute(
        "UPDATE history_record_digests SET digest=?1 WHERE source_id=?2 AND message_id=?3",
        params![tampered_digest, source.id, message_id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_record_digests SET digest=?1 WHERE source_id=?2 AND message_id=?3",
        params![digest, source.id, message_id],
    )
    .unwrap();
    c.execute(
        "DELETE FROM history_record_digests WHERE source_id=?1 AND message_id=?2",
        params![source.id, message_id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "INSERT INTO history_record_digests(source_id,message_id,digest) VALUES(?1,?2,?3)",
        params![source.id, message_id, digest],
    )
    .unwrap();
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(10))
            .unwrap()
            .data
            .len(),
        8
    );

    let wrapped: Vec<u8> = c
        .query_row(
            "SELECT wrapped_key FROM history_index_keys WHERE account_id=?1",
            [&f.account],
            |r| r.get(0),
        )
        .unwrap();
    c.execute(
        "DELETE FROM history_index_keys WHERE account_id=?1",
        [&f.account],
    )
    .unwrap();
    assert_source_corrupted(&f);
    assert_eq!(f.import().unwrap_err().code, "SOURCE_CORRUPTED");
    // No new key may be silently generated over an existing v2 source.
    assert_eq!(
        c.query_row("SELECT count(*) FROM history_index_keys", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute(
        "INSERT INTO history_index_keys(account_id,wrapped_key) VALUES(?1,?2)",
        params![f.account, wrapped],
    )
    .unwrap();
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(10))
            .unwrap()
            .data
            .len(),
        8
    );
}

#[test]
fn regression_exact_wire_limit_allows_normalized_message_id_growth_in_storage() {
    let f = Fixture::new(1, 1, 0);
    f.provider
        .regression_case
        .store(EXACT_WIRE_LIMIT_CASE, Ordering::SeqCst);
    let source = f.import().unwrap();
    assert_eq!(source.message_count, 1);
    let stats = *f.provider.stats.lock().unwrap();
    assert_eq!(stats.batches, 1);
    assert_eq!(stats.max_batch_bytes, HISTORY_BATCH_BYTES);
    assert_eq!(stats.serialized_bytes, HISTORY_BATCH_BYTES);
    let batch = f
        .repository
        .history_batch(&f.account, &source.id, 0)
        .unwrap()
        .unwrap();
    // A one-character reader id becomes a 64-character normalized id. The
    // authenticated storage budget must allow those additional 63 JSON bytes,
    // plus the AES-GCM tag, without relaxing the raw wire budget at all.
    assert_eq!(batch.ciphertext.len(), HISTORY_BATCH_BYTES + 63 + 16);
    let page = f.app.history_messages_local(&f.account, &query(2)).unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id.len(), 64);
    assert_eq!(page.data[0].conversation_id, CONVERSATION);
    let text = page.data[0].text.as_ref().unwrap();
    assert!(text.len() > HISTORY_BATCH_BYTES - 1024);
    assert!(text.bytes().all(|b| b == b'x'));
    assert!(page.next_cursor.is_none());
}

#[test]
fn regression_later_old_session_batch_keeps_newer_message_summary_and_unread_metadata() {
    let f = Fixture::new(1, 1, 0);
    f.provider
        .regression_case
        .store(LATE_OLD_SESSION_CASE, Ordering::SeqCst);
    let source = f.import().unwrap();
    assert_eq!(source.message_count, 1);
    assert_eq!(source.session_count, 1);
    assert_eq!(f.provider.stats.lock().unwrap().batches, 2);
    let sessions = f.app.history_sessions_local(&f.account).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].username, "C");
    assert_eq!(sessions[0].timestamp, 200);
    assert_eq!(sessions[0].summary.as_deref(), Some("new"));
    assert_eq!(sessions[0].unread_count, Some(7));
    let messages = f.app.history_messages_local(&f.account, &query(2)).unwrap();
    assert_eq!(messages.data.len(), 1);
    assert_eq!(messages.data[0].conversation_id, "C");
    assert_eq!(messages.data[0].timestamp, 200);
    assert_eq!(messages.data[0].text.as_deref(), Some("new"));
}

#[test]
fn batch_cipher_nonce_sequence_aad_and_missing_batch_corruption_fail_closed_on_query() {
    let f = Fixture::new(8, 2, 768);
    let source = f.import().unwrap();
    let c = f.connection();
    let batch = f
        .repository
        .history_batch(&f.account, &source.id, 0)
        .unwrap()
        .unwrap();
    let other = f
        .repository
        .history_batch(&f.account, &source.id, 1)
        .unwrap()
        .unwrap();
    let mut corrupted = batch.ciphertext.clone();
    corrupted[0] ^= 0x80;
    c.execute(
        "UPDATE history_batches SET ciphertext=?1 WHERE source_id=?2 AND sequence=0",
        params![corrupted, source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_batches SET ciphertext=?1 WHERE source_id=?2 AND sequence=0",
        params![batch.ciphertext, source.id],
    )
    .unwrap();
    let mut nonce = batch.nonce.clone();
    nonce[0] ^= 1;
    c.execute(
        "UPDATE history_batches SET nonce=?1 WHERE source_id=?2 AND sequence=0",
        params![nonce, source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    // Even a valid ciphertext+nonce pair cannot move to another sequence: the
    // sequence/source/account are authenticated in each batch's AES-GCM AAD.
    c.execute(
        "UPDATE history_batches SET nonce=?1,ciphertext=?2 WHERE source_id=?3 AND sequence=0",
        params![other.nonce, other.ciphertext, source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "UPDATE history_batches SET nonce=?1,ciphertext=?2 WHERE source_id=?3 AND sequence=0",
        params![batch.nonce, batch.ciphertext, source.id],
    )
    .unwrap();
    c.execute(
        "DELETE FROM history_batches WHERE source_id=?1 AND sequence=1",
        [&source.id],
    )
    .unwrap();
    assert_source_corrupted(&f);
    c.execute(
        "INSERT INTO history_batches(source_id,sequence,nonce,ciphertext) VALUES(?1,1,?2,?3)",
        params![source.id, other.nonce, other.ciphertext],
    )
    .unwrap();
    assert_eq!(
        f.app
            .history_messages_local(&f.account, &query(10))
            .unwrap()
            .data
            .len(),
        8
    );
}
