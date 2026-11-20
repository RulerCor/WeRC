use std::sync::Arc;
use werc_application::ports::Repository;
use werc_domain::Account;
use werc_infrastructure::SqliteRepository;

// 仅测试旧版确实存在的多个隔离域。不能通过新版登记入口造多微信。
pub fn legacy_pair(first: &str, second: &str) -> (Arc<SqliteRepository>, Account, Account) {
    let path = std::env::temp_dir().join(format!(
        "werc-legacy-fixture-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(include_str!("../../../infrastructure/migrations/0001.sql"))
        .unwrap();
    c.execute(
        "INSERT INTO accounts VALUES('fixture-current',?1,'2026-01-01')",
        [first],
    )
    .unwrap();
    c.execute(
        "INSERT INTO accounts VALUES('fixture-archive',?1,'2026-01-01')",
        [second],
    )
    .unwrap();
    drop(c);
    let repo = Arc::new(SqliteRepository::open(&path).unwrap());
    repo.select_current_connection("fixture-current", "0")
        .unwrap();
    let accounts = repo.accounts().unwrap();
    let a = accounts
        .iter()
        .find(|a| a.id == "fixture-current")
        .unwrap()
        .clone();
    let b = accounts
        .iter()
        .find(|a| a.id == "fixture-archive")
        .unwrap()
        .clone();
    (repo, a, b)
}
