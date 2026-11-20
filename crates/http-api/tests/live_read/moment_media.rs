use super::*;
#[test]
fn manual_moment_media_retains_display_row_for_offline_export() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let m = f
        .app
        .fetch_moment_media_local(
            &f.account,
            &view.view_id,
            &selection(),
            "owned-manual-media",
        )
        .unwrap();
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo, f.reader.clone());
    let rows = app
        .preserved_moments_local(&f.account, None, &Default::default())
        .unwrap();
    assert_eq!(rows.data.len(), 1);
    assert_eq!(rows.data[0].moment.anchor.id, selection().anchor.id);
    assert!(
        !app.moment_preservation_status_local(&f.account)
            .unwrap()
            .enabled
    );
    let mut zip = std::io::Cursor::new(Vec::new());
    let summary = app
        .export_moment_bundle_local(
            &f.account,
            None,
            &Default::default(),
            "owned-manual-bundle",
            &mut zip,
        )
        .unwrap();
    assert_eq!(
        (summary.rows, summary.media, summary.missing_media),
        (1, 1, 0)
    );
    assert_eq!(
        app.cached_moment_media_local(&f.account, &selection())
            .unwrap()
            .unwrap()
            .id,
        m.id
    );
}
fn selection() -> MomentMediaSelection {
    MomentMediaSelection {
        anchor: LiveMomentAnchor {
            id: (u64::MAX - 1).to_string(),
            row_id: "31".into(),
            username: "owned-moment-31".into(),
        },
        slot: 0,
        variant: MomentMediaVariant::Original,
    }
}
#[test]
fn reading_source_repairs_old_manual_media_without_redownloading() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let saved = f
        .app
        .fetch_moment_media_local(&f.account, &view.view_id, &selection(), "owned-old-media-001")
        .unwrap();
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let rows = repo
        .library_list(&f.account, "moment_preserved", None, 100)
        .unwrap();
    assert_eq!(rows.data.len(), 1);
    for row in rows.data {
        repo.library_delete(&f.account, &row.id, row.revision)
            .unwrap();
    }
    assert_eq!(
        repo.library_count(&f.account, "moment_preserved").unwrap(),
        0
    );
    let app = Application::new(repo.clone(), f.reader.clone());
    let view = f.prepare().unwrap();
    // The view belongs to f.app; use that instance to perform the authenticated read.
    f.app
        .live_moments_local(&f.account, &view.view_id, None, "owned-old-media-repair")
        .unwrap();
    assert_eq!(
        repo.library_count(&f.account, "moment_preserved").unwrap(),
        1
    );
    assert_eq!(
        app.cached_moment_media_local(&f.account, &selection())
            .unwrap()
            .unwrap()
            .sha256,
        saved.sha256
    );
}
#[test]
fn moment_media_retention_resume_backup_offline_bundle_and_encryption() {
    let f = Fixture::new();
    f.prepare().unwrap();
    f.reader.moment_media_enabled.store(true, Ordering::SeqCst);
    f.app
        .configure_moment_preservation_local(&f.account, true)
        .unwrap();
    let first = f.app.step_moment_preservation_local(&f.account).unwrap();
    assert_eq!(first.media_pending, 1);
    assert_eq!(first.media_saved, 0);
    let app = Application::new(
        Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap()),
        f.reader.clone(),
    );
    let second = app.step_moment_preservation_local(&f.account).unwrap();
    assert_eq!(second.media_pending, 0);
    assert_eq!(second.media_saved, 1);
    let done = app.step_moment_preservation_local(&f.account).unwrap();
    assert!(!done.scanning);
    assert_eq!(done.saved, 31);
    let m = app
        .cached_moment_media_local(&f.account, &selection())
        .unwrap()
        .unwrap();
    let (_, bytes) = app.moment_media_contents_local(&f.account, &m.id).unwrap();
    assert_eq!(
        &**bytes,
        include_bytes!("../../../../tests/fixtures/owned-image.png")
    );
    app.configure_moment_preservation_local(&f.account, false)
        .unwrap();
    std::fs::rename(&f.database, f.database.with_extension("owned-moved")).unwrap();
    let mut backup = Vec::new();
    assert_eq!(
        app.backup_library_local(
            &f.account,
            "owned-media-password",
            "owned-media-backup",
            &mut backup
        )
        .unwrap(),
        33
    );
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let account = repo.setup_account("owned restore").unwrap().id;
    let restored = Application::new(repo, f.reader.clone());
    assert_eq!(
        restored
            .restore_library_local(
                &account,
                "owned-media-password",
                "owned-media-restore",
                &mut std::io::Cursor::new(backup)
            )
            .unwrap(),
        33
    );
    let imported = restored
        .cached_moment_media_local(&account, &selection())
        .unwrap()
        .unwrap();
    assert_ne!(m.id, imported.id);
    assert_eq!(
        restored
            .moment_media_contents_local(&account, &imported.id)
            .unwrap()
            .1,
        bytes
    );
    let mut zip = std::io::Cursor::new(Vec::new());
    let summary = restored
        .export_moment_bundle_local(
            &account,
            None,
            &Default::default(),
            "owned-media-bundle",
            &mut zip,
        )
        .unwrap();
    assert_eq!(
        (summary.rows, summary.media, summary.missing_media),
        (31, 1, 30)
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip.into_inner())).unwrap();
    use std::io::Read;
    let mut html = String::new();
    archive
        .by_name("index.html")
        .unwrap()
        .read_to_string(&mut html)
        .unwrap();
    assert!(html.contains("media/"));
    assert!(!html.contains("https://"));
    let mut manifest = String::new();
    archive
        .by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let path = manifest["media"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|v| v["path"].as_str())
        .unwrap();
    assert!(path.starts_with("media/"));
    let mut read = Vec::new();
    archive
        .by_name(path)
        .unwrap()
        .read_to_end(&mut read)
        .unwrap();
    assert_eq!(read, *bytes);
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(!disk.windows(32).any(|b| b == &bytes[..32]));
}
#[test]
fn moment_media_identity_change_and_cancellation_do_not_commit() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.reader.change.store(true, Ordering::SeqCst);
    assert_eq!(
        f.app
            .fetch_moment_media_local(
                &f.account,
                &view.view_id,
                &selection(),
                "owned-media-changed"
            )
            .err()
            .unwrap()
            .code,
        "CURSOR_STALE"
    );
    assert_eq!(f.repo.library_count(&f.account, "moment_media").unwrap(), 0);
    assert_eq!(f.repo.library_count(&f.account, "media_chunk").unwrap(), 0);
    f.reader.change.store(false, Ordering::SeqCst);
    let view = f.prepare().unwrap();
    f.app.cancel_browse_local("owned-media-canceled").unwrap();
    assert_eq!(
        f.app
            .fetch_moment_media_local(
                &f.account,
                &view.view_id,
                &selection(),
                "owned-media-canceled"
            )
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.repo.library_count(&f.account, "media_chunk").unwrap(), 0);
}

#[tokio::test]
async fn moment_media_http_permissions_content_and_bundle() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let mut id = String::new();
    for (tail, scopes, expected) in [
        ("media", vec!["moments:read"], 403),
        ("media", vec!["moments:read", "media:read"], 200),
        ("media/content", vec!["moments:read"], 403),
        ("media/content", vec!["moments:read", "media:read"], 200),
        ("bundle", vec!["moments:read", "media:read"], 403),
        (
            "bundle",
            vec!["moments:read", "media:read", "exports:create"],
            200,
        ),
    ] {
        let token = f
            .app
            .issue_token_local(
                "owned media api",
                vec![f.account.clone()],
                scopes.into_iter().map(str::to_owned).collect(),
                1,
            )
            .unwrap();
        let body = match tail {
            "media" => {
                serde_json::json!({"account_id":f.account,"view_id":view.view_id,"selection":selection(),"request_id":"owned-http-moment-media"})
            }
            "media/content" => serde_json::json!({"account_id":f.account,"id":id}),
            _ => {
                serde_json::json!({"account_id":f.account,"view_id":view.view_id,"query":{},"request_id":"owned-http-moment-bundle"})
            }
        };
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/moments/{tail}"))
                    .header("Authorization", format!("Bearer {}", token.secret))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected, "{tail}");
        if expected == 200 {
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            match tail {
                "media" => {
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    id = value["id"].as_str().unwrap().into();
                    assert!(!String::from_utf8_lossy(&bytes).contains("token"));
                }
                "media/content" => assert_eq!(
                    &*bytes,
                    include_bytes!("../../../../tests/fixtures/owned-image.png")
                ),
                _ => assert!(bytes.starts_with(b"PK")),
            }
        }
    }
}
