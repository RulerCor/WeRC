use super::*;
use werc_application::directory_export::*;
#[test]
fn contact_avatar_survives_reopen_backup_and_relative_directory_bundle(){
    use std::io::{Read,Cursor};
    let f=Fixture::new();let view=f.prepare().unwrap();
    let avatar=f.app.contact_avatar_local(&f.account,&view.view_id,"owned-contact-201","owned-avatar-read").unwrap();
    let bytes=avatar.bytes().unwrap();
    let app=Application::new(f.repo.clone(),f.reader.clone());
    let same=app.contact_avatar_local(&f.account,&view.view_id,"owned-contact-201","owned-avatar-other-view");
    assert!(same.is_err()); // View leases belong to the application that created them.
    let selection=DirectorySelection::Contacts{search_query:None,categories:vec![]};
    let output=f.app.export_live_directory_local(&f.account,&view.view_id,&selection,DirectoryFormat::Zip,"owned-avatar-export",Ok).unwrap();
    assert_eq!(output.summary.avatars_written,1);
    let mut zip=zip::ZipArchive::new(Cursor::new(output.bytes.to_vec())).unwrap();
    let mut text=String::new();zip.by_name("avatars.json").unwrap().read_to_string(&mut text).unwrap();
    let manifest:serde_json::Value=serde_json::from_str(&text).unwrap();
    let row=manifest.as_array().unwrap().iter().find(|v|v["status"]=="saved").unwrap();let path=row["path"].as_str().unwrap();assert!(path.starts_with("avatars/")&&!path.contains(".."));
    let mut restored=vec![];zip.by_name(path).unwrap().read_to_end(&mut restored).unwrap();assert_eq!(restored,*bytes);
    let mut backup=vec![];f.app.backup_library_local(&f.account,"owned-avatar-password","owned-avatar-backup",&mut backup).unwrap();
    let other=Fixture::new();other.app.restore_library_local(&other.account,"owned-avatar-password","owned-avatar-restore",&mut Cursor::new(backup)).unwrap();
    let parent=other.prepare().unwrap();
    let output=other.app.export_live_directory_local(&other.account,&parent.view_id,&selection,DirectoryFormat::Zip,"owned-avatar-restored-export",Ok).unwrap();
    assert_eq!(output.summary.avatars_written,1);
    assert!(f.app.contact_avatar_local(&f.account,&view.view_id,"unknown-contact","owned-avatar-missing").is_err());
}
#[test]
fn directory_export_reads_all_pages_preserves_unknown_members_and_filters_categories() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let selection = DirectorySelection::Contacts {
        search_query: None,
        categories: vec![],
    };
    let output = f
        .app
        .export_live_directory_local(
            &f.account,
            &parent.view_id,
            &selection,
            DirectoryFormat::Json,
            "owned-directory-json",
            Ok,
        )
        .unwrap();
    assert_eq!(output.summary.rows, 201);
    assert_eq!(output.summary.pages, 2);
    assert_eq!(output.summary.avatars_written, 0);
    assert!(!output.summary.complete_server_directory);
    let json: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
    assert_eq!(
        json["rows"][200]["record"]["anchor"]["username"],
        "owned-contact-201"
    );
    let selection = DirectorySelection::Group {
        username: "owned@chatroom".into(),
    };
    let output = f
        .app
        .export_live_directory_local(
            &f.account,
            &parent.view_id,
            &selection,
            DirectoryFormat::Json,
            "owned-directory-group",
            Ok,
        )
        .unwrap();
    assert_eq!(output.summary.rows, 201);
    let json: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
    assert!(json["rows"][4]["record"]["anchor"]["username"].is_null());
    assert_eq!(json["rows"][200]["record"]["anchor"]["row_id"], "201");
    let selection = DirectorySelection::Contacts {
        search_query: None,
        categories: vec![DirectoryCategory::Group],
    };
    let output = f
        .app
        .export_live_directory_local(
            &f.account,
            &parent.view_id,
            &selection,
            DirectoryFormat::Csv,
            "owned-directory-category",
            Ok,
        )
        .unwrap();
    assert_eq!(output.summary.rows, 0);
    assert_eq!(output.summary.pages, 2);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[test]
fn directory_export_cancellation_and_source_changes_never_invoke_writer() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let selection = DirectorySelection::Contacts {
        search_query: None,
        categories: vec![],
    };
    let written = AtomicBool::new(false);
    f.app.cancel_browse_local("owned-directory-cancel").unwrap();
    assert_eq!(
        f.app
            .export_live_directory_local(
                &f.account,
                &parent.view_id,
                &selection,
                DirectoryFormat::Csv,
                "owned-directory-cancel",
                |_| {
                    written.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .export_live_directory_local(
                &f.account,
                &parent.view_id,
                &selection,
                DirectoryFormat::Json,
                "owned-directory-changed",
                |_| {
                    written.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(!written.load(Ordering::SeqCst));
}
#[tokio::test]
async fn directory_export_http_requires_read_and_export_scopes_and_returns_attachment() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str| {
        Request::builder().method("POST").uri("/api/v1/contacts/live/export").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").body(Body::from(serde_json::json!({"account_id":f.account,"view_id":parent.view_id,"selection":{"kind":"contacts","search_query":null,"categories":[]},"format":"json","request_id":"owned-directory-http"}).to_string())).unwrap()
    };
    for scope in ["contacts:read", "exports:create"] {
        let token = f
            .app
            .issue_token_local(
                "owned missing scope",
                vec![f.account.clone()],
                vec![scope.into()],
                1,
            )
            .unwrap();
        assert_eq!(
            router
                .clone()
                .oneshot(request(&token.secret))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let token = f
        .app
        .issue_token_local(
            "owned directory export",
            vec![f.account.clone()],
            vec!["contacts:read".into(), "exports:create".into()],
            1,
        )
        .unwrap();
    let response = router.oneshot(request(&token.secret)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-werc-records"], "201");
    assert_eq!(response.headers()["x-werc-pages"], "2");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 201);
}
