use crate::{live::ReadView, live_protocol::*};
use std::path::Path;

pub(crate) fn read(
    path: &Path,
    key: &str,
    selection: &LiveCalendarSelection,
) -> Result<LiveCalendarWindow, &'static str> {
    if !selection.valid() {
        return Err("LIVE_CALENDAR_INVALID");
    }
    let view = ReadView::open(path, key)?;
    let c = &view.connection;
    let table = crate::window::conversation_table(c, &selection.username)?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [&table],
            |r| r.get(0),
        )
        .map_err(|_| "LIVE_READ_FAILED")?;
    if !exists {
        return Err("LIVE_CONVERSATION_NOT_IN_SOURCE");
    }
    let mut info = c
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    let columns = info
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    let time = crate::window::column(&columns, &["create_time", "createTime"])?;
    let mut statement = c
        .prepare(&format!(
            "SELECT count(*) FROM \"{table}\" WHERE {time}>=?1 AND {time}<?2"
        ))
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    let mut counts = Vec::with_capacity(selection.days.len());
    for day in &selection.days {
        view.check_deadline()?;
        let count: i64 = statement
            .query_row(
                rusqlite::params![day.from_timestamp, day.until_timestamp],
                |r| r.get(0),
            )
            .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?;
        counts.push(u64::try_from(count).map_err(|_| "LIVE_PROTOCOL_INVALID")?);
    }
    view.check_deadline()?;
    c.execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    let result = LiveCalendarWindow {
        counts,
        read_only: true,
        readonly_shm: true,
    };
    if !result.valid(selection) {
        return Err("LIVE_PROTOCOL_INVALID");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_calendar_matches_half_open_message_windows_and_variable_days() {
        crate::live::prepare_memory().unwrap();
        let root = std::env::temp_dir().join(format!(
            "werc-calendar-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("owned.sqlite3");
        let key = "5b".repeat(32);
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(&format!("PRAGMA key=\"x'{key}'\"; PRAGMA cipher_compatibility=4; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE {FILEHELPER_TABLE}(local_id INTEGER PRIMARY KEY,create_time INTEGER,local_type INTEGER,server_id INTEGER,message_content BLOB)")).unwrap();
        let start = 1_700_000_000;
        let bounds = [
            start,
            start + 23 * 3600,
            start + 48 * 3600,
            start + 72 * 3600,
        ];
        for (i, time) in [
            start - 1,
            start,
            start,
            bounds[1] - 1,
            bounds[1],
            bounds[2] - 1,
            bounds[3],
        ]
        .iter()
        .enumerate()
        {
            c.execute(
                &format!(
                    "INSERT INTO {FILEHELPER_TABLE} VALUES(?1,?2,1,9007199254740993+?1,'owned')"
                ),
                rusqlite::params![i + 1, time],
            )
            .unwrap();
        }
        let selection = LiveCalendarSelection {
            username: "filehelper".into(),
            days: bounds
                .windows(2)
                .map(|p| LiveCalendarDay {
                    from_timestamp: p[0],
                    until_timestamp: p[1],
                })
                .collect(),
        };
        let result = read(&path, &key, &selection).unwrap();
        assert_eq!(result.counts, [3, 2, 0]);
        for (day, count) in selection.days.iter().zip(result.counts) {
            let mut query = LiveMessageQuery::default();
            query.from_timestamp = Some(day.from_timestamp);
            query.until_timestamp = Some(day.until_timestamp);
            let selected = LiveConversationSelection {
                username: "filehelper".into(),
                query: Some(query),
                before: None,
                include_anchor: false,
                newer_context: false,
            };
            let window =
                crate::window::read_selected(&path, &key, 60, Some(&selected), None).unwrap();
            assert_eq!(window.rows.len() as u64, count);
        }
        let mut invalid = selection.clone();
        invalid.days[1].from_timestamp += 1;
        assert!(matches!(
            read(&path, &key, &invalid),
            Err("LIVE_CALENDAR_INVALID")
        ));
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
}
