//! Search real message tables, including conversations absent from the session cache.
use crate::{live::ReadView, live_protocol::*};
use std::path::Path;

pub(crate) fn read(
    path: &Path,
    key: &str,
    selection: &LiveSearchSelection,
) -> Result<LiveSearchWindow, &'static str> {
    if !selection.valid() {
        return Err("LIVE_QUERY_INVALID");
    }
    let view = ReadView::open(path, key)?;
    let c = &view.connection;
    c.create_scalar_function(
        "werc_message_table",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            use md5::{Digest, Md5};
            let name: String = context.get(0)?;
            Ok(format!("Msg_{:x}", Md5::digest(name.as_bytes())))
        },
    )
    .map_err(|_| "LIVE_QUERY_UNAVAILABLE")?;
    let mut statement=c.prepare("SELECT user_name FROM (SELECT DISTINCT user_name FROM Name2Id WHERE typeof(user_name)='text' AND length(CAST(user_name AS BLOB)) BETWEEN 1 AND 256 UNION SELECT 'filehelper') WHERE (?1 IS NULL OR user_name COLLATE BINARY>?1 OR (?2 AND user_name COLLATE BINARY=?1)) AND EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=werc_message_table(user_name)) ORDER BY user_name COLLATE BINARY LIMIT 17")
        .map_err(|_| "LIVE_IDENTITY_SCHEMA_UNSUPPORTED")?;
    let names = statement
        .query_map(
            rusqlite::params![
                selection.after.as_ref().map(|p| p.username.as_str()),
                selection.after.as_ref().is_some_and(|p| p.before.is_some())
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "LIVE_READ_FAILED")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?;
    if selection
        .after
        .as_ref()
        .is_some_and(|p| p.before.is_some() && names.first() != Some(&p.username))
    {
        return Err("CURSOR_STALE");
    }
    let mut result = LiveSearchWindow {
        rows: vec![],
        next: None,
        scanned_conversations: 0,
        read_only: true,
        readonly_shm: true,
    };
    for (index, username) in names.iter().take(LIVE_SEARCH_MAX_CONVERSATIONS).enumerate() {
        view.check_deadline()?;
        if !valid_live_username(username) {
            return Err("LIVE_IDENTITY_SCHEMA_UNSUPPORTED");
        }
        result.scanned_conversations += 1;
        let before = selection
            .after
            .as_ref()
            .filter(|p| p.username == *username)
            .and_then(|p| p.before.clone());
        let selected = LiveConversationSelection {
            username: username.clone(),
            before,
            include_anchor: false,
            newer_context: false,
            query: Some(selection.query.clone()),
        };
        let remaining = LIVE_SEARCH_MAX_ROWS - result.rows.len();
        match crate::window::read_in_view(&view, remaining, Some(&selected), None) {
            Ok(window) => {
                if window.has_older {
                    let last = window.rows.last().ok_or("LIVE_PROTOCOL_INVALID")?;
                    result.next = Some(LiveSearchPosition {
                        username: username.clone(),
                        before: Some(LiveAnchor {
                            timestamp: last.timestamp,
                            local_id: last.local_id.clone(),
                            server_id: last.server_id.clone(),
                        }),
                    });
                }
                result
                    .rows
                    .extend(window.rows.into_iter().map(|message| LiveSearchHit {
                        username: username.clone(),
                        message,
                    }));
            }
            Err("LIVE_CONVERSATION_NOT_IN_SOURCE") if selected.before.is_none() => {}
            Err(e) => return Err(e),
        }
        if result.next.is_some() {
            break;
        }
        if result.rows.len() == LIVE_SEARCH_MAX_ROWS || index + 1 == LIVE_SEARCH_MAX_CONVERSATIONS {
            if index + 1 < names.len() {
                result.next = Some(LiveSearchPosition {
                    username: username.clone(),
                    before: None,
                });
            }
            break;
        }
    }
    view.check_deadline()?;
    c.execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    if !result.valid(selection) {
        return Err("LIVE_PROTOCOL_INVALID");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_search_reaches_hidden_old_conversations_and_resumes_empty_batches() {
        use md5::{Digest, Md5};
        crate::live::prepare_memory().unwrap();
        let root = std::env::temp_dir().join(format!(
            "werc-search-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("owned.sqlite3");
        let key = "5a".repeat(32);
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(&format!("PRAGMA key=\"x'{key}'\"; PRAGMA cipher_compatibility=4; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE Name2Id(user_name TEXT);")).unwrap();
        for n in 0..40 {
            let username = format!("empty-{n:03}");
            c.execute("INSERT INTO Name2Id VALUES(?1)", [&username])
                .unwrap();
            let table = format!("Msg_{:x}", Md5::digest(username.as_bytes()));
            c.execute_batch(&format!("CREATE TABLE {table}(local_id INTEGER PRIMARY KEY,create_time INTEGER,local_type INTEGER,server_id INTEGER,message_content BLOB)")).unwrap();
        }
        for username in ["filehelper", "z-hidden-friend"] {
            c.execute("INSERT INTO Name2Id VALUES(?1)", [username])
                .unwrap();
            c.execute("INSERT INTO Name2Id VALUES(?1)", [username])
                .unwrap(); // duplicate mappings must not duplicate hits
            let table = format!("Msg_{:x}", Md5::digest(username.as_bytes()));
            c.execute_batch(&format!("CREATE TABLE {table}(local_id INTEGER PRIMARY KEY,create_time INTEGER,local_type INTEGER,server_id INTEGER,message_content BLOB)")).unwrap();
            for n in 1..=125 {
                c.execute(
                    &format!("INSERT INTO {table} VALUES(?1,1700000000,1,9007199254740992+?1,?2)"),
                    rusqlite::params![n, if n == 1 { "old needle" } else { "ordinary" }],
                )
                .unwrap();
            }
        }
        let mut selection = LiveSearchSelection {
            query: LiveMessageQuery::default(),
            after: None,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut empty = 0;
        loop {
            let result = read(&path, &key, &selection).unwrap();
            assert!(result.valid(&selection));
            if result.rows.is_empty() {
                empty += 1;
            }
            for hit in &result.rows {
                assert!(seen.insert((hit.username.clone(), hit.message.local_id.clone())));
            }
            let Some(next) = result.next else { break };
            selection.after = Some(next);
        }
        assert_eq!(seen.len(), 250);
        assert_eq!(empty, 2);
        selection = LiveSearchSelection {
            query: LiveMessageQuery::default(),
            after: None,
        };
        selection.query.text = Some("NEEDLE".into());
        let mut found = 0;
        loop {
            let result = read(&path, &key, &selection).unwrap();
            for hit in &result.rows {
                assert_eq!(hit.message.local_id, "1");
                found += 1;
            }
            let Some(next) = result.next else { break };
            selection.after = Some(next);
        }
        assert_eq!(found, 2);
        let first = read(
            &path,
            &key,
            &LiveSearchSelection {
                query: LiveMessageQuery::default(),
                after: Some(LiveSearchPosition {
                    username: "empty-039".into(),
                    before: None,
                }),
            },
        )
        .unwrap();
        let after = first.next.unwrap();
        c.execute(
            &format!("DELETE FROM {FILEHELPER_TABLE} WHERE local_id=?1"),
            [after
                .before
                .as_ref()
                .unwrap()
                .local_id
                .parse::<i64>()
                .unwrap()],
        )
        .unwrap();
        assert!(matches!(
            read(
                &path,
                &key,
                &LiveSearchSelection {
                    query: LiveMessageQuery::default(),
                    after: Some(after)
                }
            ),
            Err("CURSOR_STALE")
        ));
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
}
