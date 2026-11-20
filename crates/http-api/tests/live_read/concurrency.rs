use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn blocked_contacts(
    f: &Fixture,
    view: &str,
) -> std::thread::JoinHandle<AppResult<werc_application::live_contacts::LiveContactPage>> {
    let before = f.reader.reads.load(Ordering::SeqCst);
    f.reader.wait.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let view = view.to_owned();
    let thread = std::thread::spawn(move || {
        app.live_contacts_local(&account, &view, None, "owned-concurrent-contacts")
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.reader.reads.load(Ordering::SeqCst) == before {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    thread
}

#[test]
fn independent_contacts_and_sessions_reach_provider_simultaneously() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let first = blocked_contacts(&f, &parent.view_id);
    let expected = f.reader.reads.load(Ordering::SeqCst) + 1;
    let app = f.app.clone();
    let account = f.account.clone();
    let view = parent.view_id;
    let (tx, rx) = mpsc::channel();
    let second = std::thread::spawn(move || {
        tx.send(app.live_sessions_local(&account, &view, None, "owned-concurrent-sessions"))
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.reader.reads.load(Ordering::SeqCst) < expected && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let overlapped = f.reader.reads.load(Ordering::SeqCst) >= expected;
    f.reader.wait.store(false, Ordering::SeqCst);
    assert!(
        overlapped,
        "independent source reads must reach provider concurrently"
    );
    assert!(!first.join().unwrap().unwrap().rows.is_empty());
    assert!(
        !rx.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .rows
            .is_empty()
    );
    second.join().unwrap();
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

fn blocked_prepare(
    f: &Fixture,
) -> std::thread::JoinHandle<AppResult<werc_application::live_view::LivePage>> {
    let before = f.reader.reads.load(Ordering::SeqCst);
    f.reader.wait.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let database = f.database.to_string_lossy().to_string();
    let thread = std::thread::spawn(move || {
        app.prepare_live_filehelper_local(&account, &database, "owned-exclusive-prepare", true)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while f.reader.reads.load(Ordering::SeqCst) == before {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    thread
}

#[test]
fn queued_read_cancels_before_active_source_is_released_and_does_not_read() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let first = blocked_prepare(&f);
    let reads = f.reader.reads.load(Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let view = parent.view_id;
    let (tx, rx) = mpsc::channel();
    let second = std::thread::spawn(move || {
        tx.send(app.live_sessions_local(&account, &view, None, "owned-concurrent-cancel"))
            .unwrap();
    });
    let early = rx.recv_timeout(Duration::from_millis(150));
    f.app
        .cancel_browse_local("owned-concurrent-cancel")
        .unwrap();
    let result = if early.is_ok() {
        early.unwrap()
    } else {
        rx.recv_timeout(Duration::from_secs(3)).unwrap()
    };
    let after = f.reader.reads.load(Ordering::SeqCst);
    f.reader.wait.store(false, Ordering::SeqCst);
    assert_eq!(result.err().unwrap().code, "QUERY_CANCELED");
    assert_eq!(
        reads, after,
        "queued cancellation must not reach the source provider"
    );
    first.join().unwrap().unwrap();
    second.join().unwrap();
    assert!(
        !f.app
            .live_sessions_local(&f.account, &view_for(&f), None, "owned-after-cancel-query")
            .unwrap()
            .rows
            .is_empty()
    );
}

fn view_for(f: &Fixture) -> String {
    f.prepare().unwrap().view_id
}

#[test]
fn queued_read_rechecks_connection_after_waiting() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let first = blocked_prepare(&f);
    let app = f.app.clone();
    let account = f.account.clone();
    let view = parent.view_id;
    let (tx, rx) = mpsc::channel();
    let second = std::thread::spawn(move || {
        tx.send(app.live_sessions_local(&account, &view, None, "owned-concurrent-changed"))
            .unwrap();
    });
    let early = rx.recv_timeout(Duration::from_millis(150));
    f.reader.context.lock().unwrap().epoch = "changed-while-waiting".into();
    f.reader.wait.store(false, Ordering::SeqCst);
    assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
    assert!(first.join().unwrap().is_err());
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
    second.join().unwrap();
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
