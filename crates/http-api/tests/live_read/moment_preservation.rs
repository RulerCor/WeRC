use super::*;

#[test]
fn moment_preservation_resumes_keeps_copies_after_source_disappears_and_exports_all_pages() {
    let f = Fixture::new();
    f.prepare().unwrap();
    assert!(
        !f.app
            .moment_preservation_status_local(&f.account)
            .unwrap()
            .enabled
    );
    f.app
        .configure_moment_preservation_local(&f.account, true)
        .unwrap();
    let first = f.app.step_moment_preservation_local(&f.account).unwrap();
    assert_eq!(first.saved, 30);
    assert!(first.scanning);
    f.reader.context.lock().unwrap().epoch = "reconnected-preservation".into();
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo, f.reader.clone());
    let done = app.step_moment_preservation_local(&f.account).unwrap();
    assert_eq!(done.saved, 31);
    assert!(!done.scanning);
    assert!(done.last_completed.is_some());
    app.configure_moment_preservation_local(&f.account, false)
        .unwrap();
    let reads = f.reader.reads.load(Ordering::SeqCst);
    assert!(
        !app.step_moment_preservation_local(&f.account)
            .unwrap()
            .enabled
    );
    assert_eq!(reads, f.reader.reads.load(Ordering::SeqCst));
    let page = app
        .preserved_moments_local(&f.account, None, &MomentQuery::default())
        .unwrap();
    assert_eq!(page.data.len(), 30);
    assert_eq!(
        app.preserved_moments_local(&f.account, page.next_cursor, &MomentQuery::default())
            .unwrap()
            .data
            .len(),
        1
    );
    // Offline viewing and export do not need the original file to remain present.
    std::fs::rename(&f.database, f.database.with_extension("owned-moved")).unwrap();
    for format in [
        werc_application::live_moments::MomentExportFormat::Json,
        werc_application::live_moments::MomentExportFormat::Html,
        werc_application::live_moments::MomentExportFormat::Md,
        werc_application::live_moments::MomentExportFormat::Txt,
    ] {
        let result = app
            .export_preserved_moments_local(
                &f.account,
                &MomentQuery::default(),
                format,
                "owned-preserved-export",
            )
            .unwrap();
        assert_eq!(result.summary.rows, 31);
        assert_eq!(result.summary.missing_media, 31);
        assert!(String::from_utf8_lossy(&result.bytes).contains("自有朋友圈"));
    }
    assert_eq!(reads, f.reader.reads.load(Ordering::SeqCst));
    let mut backup = Vec::new();
    assert_eq!(
        app.backup_library_local(
            &f.account,
            "owned-preservation-password",
            "owned-preservation-backup",
            &mut backup
        )
        .unwrap(),
        31
    );
    let restore_repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let restored = restore_repo
        .setup_account("owned restored moments")
        .unwrap()
        .id;
    let restore_app = Application::new(restore_repo, f.reader.clone());
    assert_eq!(
        restore_app
            .restore_library_local(
                &restored,
                "owned-preservation-password",
                "owned-preservation-restore",
                &mut std::io::Cursor::new(backup)
            )
            .unwrap(),
        31
    );
    let imported = restore_app
        .moment_preservation_status_local(&restored)
        .unwrap();
    assert_eq!(imported.saved, 31);
    assert!(!imported.enabled);
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !disk
            .windows("自有朋友圈".len())
            .any(|b| b == "自有朋友圈".as_bytes())
    );
}
#[test]
fn moment_preservation_does_not_duplicate_on_rescan_or_publish_a_changed_identity() {
    let f = Fixture::new();
    f.prepare().unwrap();
    f.app
        .configure_moment_preservation_local(&f.account, true)
        .unwrap();
    f.app.step_moment_preservation_local(&f.account).unwrap();
    f.app.step_moment_preservation_local(&f.account).unwrap();
    f.app
        .configure_moment_preservation_local(&f.account, true)
        .unwrap();
    assert_eq!(
        f.app
            .step_moment_preservation_local(&f.account)
            .unwrap()
            .saved,
        31
    );
    assert_eq!(
        f.app
            .step_moment_preservation_local(&f.account)
            .unwrap()
            .saved,
        31
    );
    f.app
        .configure_moment_preservation_local(&f.account, true)
        .unwrap();
    f.reader.change.store(true, Ordering::SeqCst);
    let failed = f.app.step_moment_preservation_local(&f.account).unwrap();
    assert_eq!(failed.saved, 31);
    assert_eq!(failed.error.as_deref(), Some("CONNECTION_CHANGED"));
}
