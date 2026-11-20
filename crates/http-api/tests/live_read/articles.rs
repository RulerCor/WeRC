use super::*;
use werc_application::{article_jobs::*, articles::*};
fn key() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn spec() -> ArticlePlanSpec {
    ArticlePlanSpec {
        title: "自动化测试文章计划".into(),
        publishers: vec![],
        cache_images: false,
        urls: vec![
            "https://mp.weixin.qq.com/s/owned".into(),
            "https://mp.weixin.qq.com/s/missing".into(),
        ],
        interval_seconds: 3600,
        next_run: 0,
        enabled: true,
        formats: vec!["html".into()],
        directory: "WeRC-自动化测试".into(),
    }
}
#[test]
fn article_offline_images_bundle_and_portable_backup(){
    use std::io::{Cursor,Read};
    let f=Fixture::new();
    let p=f.app.fetch_article_local(&f.account,"https://mp.weixin.qq.com/s/owned",&key()).unwrap();
    let a=f.app.confirm_article_local(&f.account,&p.id,p.revision,true).unwrap();
    let before=f.app.export_articles_local(&f.account,&[a.id.clone()],ArticleExportFormat::Zip,&key()).unwrap();
    assert_eq!(before.external_images,1);
    let m=f.app.cache_article_image_local(&f.account,&a.id,0,&key()).unwrap();
    assert_eq!(f.app.cache_article_image_local(&f.account,&a.id,0,&key()).unwrap().sha256,m.sha256);
    let app=Application::new(f.repo.clone(),f.reader.clone());
    let (_,bytes)=app.article_image_contents_local(&f.account,&m.id).unwrap();
    werc_domain::image::canonical_png(&bytes).unwrap();
    let bundle=app.export_articles_local(&f.account,&[a.id.clone()],ArticleExportFormat::Zip,&key()).unwrap();
    assert_eq!(bundle.external_images,0);
    let mut zip=zip::ZipArchive::new(Cursor::new(bundle.bytes)).unwrap();
    let mut image=vec![];zip.by_name("article-0001/images/0001.png").unwrap().read_to_end(&mut image).unwrap();
    assert_eq!(image,*bytes);
    let mut html=String::new();zip.by_name("article-0001/article.html").unwrap().read_to_string(&mut html).unwrap();
    assert!(html.contains("src=\"images/0001.png\""));
    let mut backup=vec![];app.backup_library_local(&f.account,"owned-password",&key(),&mut backup).unwrap();
    let imported_repo=Arc::new(SqliteRepository::in_memory().unwrap());
    let other=imported_repo.register_account("article backup").unwrap().id;
    let app=Application::new(imported_repo,f.reader.clone());
    app.restore_library_local(&other,"owned-password",&key(),&mut Cursor::new(backup)).unwrap();
    let restored=app.cached_article_image_local(&other,&m.url).unwrap().unwrap();
    assert_ne!(restored.id,m.id);assert_eq!(app.article_image_contents_local(&other,&restored.id).unwrap().1.as_slice(),bytes.as_slice());
}
#[test]
fn article_preview_save_discard_formats_backup_and_reopen() {
    let f = Fixture::new();
    let first = f
        .app
        .fetch_article_local(
            &f.account,
            "https://mp.weixin.qq.com/s/owned?credential=discarded",
            &key(),
        )
        .unwrap();
    assert!(first.record.saved.is_none());
    assert!(first.record.pending.is_some());
    assert_eq!(
        f.app
            .export_articles_local(
                &f.account,
                std::slice::from_ref(&first.id),
                ArticleExportFormat::Json,
                &key()
            )
            .err()
            .unwrap()
            .code,
        "ARTICLE_SAVE_REQUIRED"
    );
    let saved = f
        .app
        .confirm_article_local(&f.account, &first.id, first.revision, true)
        .unwrap();
    assert!(
        f.app
            .confirm_article_local(&f.account, &first.id, first.revision, false)
            .is_err()
    );
    f.reader.body.lock().unwrap().push_str("新的预览");
    let pending = f
        .app
        .fetch_article_local(&f.account, "https://mp.weixin.qq.com/s/owned", &key())
        .unwrap();
    assert_eq!(pending.id, saved.id);
    assert!(
        pending
            .record
            .pending
            .as_ref()
            .unwrap()
            .text
            .contains("新的预览")
    );
    let app = Application::new(f.repo.clone(), f.reader.clone());
    assert!(
        app.article_local(&f.account, &first.id)
            .unwrap()
            .record
            .pending
            .is_some()
    );
    let discarded = app
        .confirm_article_local(&f.account, &first.id, pending.revision, false)
        .unwrap();
    assert!(!discarded.record.saved.unwrap().text.contains("新的预览"));
    for format in [
        ArticleExportFormat::Json,
        ArticleExportFormat::Md,
        ArticleExportFormat::Html,
        ArticleExportFormat::Xlsx,
    ] {
        let out = app
            .export_articles_local(&f.account, std::slice::from_ref(&first.id), format, &key())
            .unwrap();
        assert_eq!(out.rows, 1);
        assert_eq!(out.external_images, 1);
        assert!(!out.bytes.is_empty());
        if matches!(format, ArticleExportFormat::Html) {
            assert!(!String::from_utf8(out.bytes).unwrap().contains("<script"));
        }
    }
    let mut backup = vec![];
    app.backup_library_local(&f.account, "owned-password", &key(), &mut backup)
        .unwrap();
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let account = repo.setup_account("restored").unwrap().id;
    let restored = Application::new(repo, f.reader.clone());
    restored
        .restore_library_local(
            &account,
            "owned-password",
            &key(),
            &mut std::io::Cursor::new(backup),
        )
        .unwrap();
    let imported = restored.articles_local(&account, None).unwrap();
    assert_eq!(imported.data.len(), 1);
    assert!(imported.data[0].record.saved.is_some());
    assert!(imported.data[0].record.owner_run.is_none());
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !disk
            .windows("自有样本".len())
            .any(|b| b == "自有样本".as_bytes())
    );
}
#[test]
fn article_plan_checkpoint_partial_confirmation_cancel_resume_and_archive() {
    let f = Fixture::new();
    let p = f
        .app
        .save_article_plan_local(&f.account, None, 0, spec())
        .unwrap();
    let run = f
        .app
        .begin_article_run_local(&f.account, &p.id, false)
        .unwrap();
    assert_eq!(
        run.id,
        f.app
            .begin_article_run_local(&f.account, &p.id, true)
            .unwrap()
            .id
    );
    let one = f.app.step_article_run_local(&f.account, &run.id).unwrap();
    assert_eq!(one.run.results.len(), 1);
    let article = one.run.results[0].id.as_ref().unwrap();
    assert_eq!(
        f.app
            .confirm_article_local(&f.account, article, 1, true)
            .err()
            .unwrap()
            .code,
        "BUSY"
    );
    f.app
        .article_job_action_local(&f.account, JobAction::Cancel { id: run.id.clone() })
        .unwrap();
    let app = Application::new(f.repo.clone(), f.reader.clone());
    app.article_job_action_local(&f.account, JobAction::Resume { id: run.id.clone() })
        .unwrap();
    let pending = app.step_article_run_local(&f.account, &run.id).unwrap();
    assert_eq!(pending.run.state, "pending");
    assert_eq!(pending.run.results.len(), 2);
    assert_eq!(
        pending.run.results[1].error.as_ref().unwrap().code,
        "ARTICLE_LOGIN_REQUIRED"
    );
    assert!(
        app.confirm_article_run_local(&f.account, &run.id, one.revision, true)
            .is_err()
    );
    let confirmed = app
        .confirm_article_run_local(&f.account, &run.id, pending.revision, true)
        .unwrap();
    assert_eq!(confirmed.run.state, "exporting");
    assert!(
        app.finish_article_run_local(&f.account, &run.id, "succeeded", None)
            .is_err()
    );
    app.article_artifact_local(
        &f.account,
        &run.id,
        werc_application::export_plans::ExportArtifact {
            format: "html".into(),
            path: "WeRC-自动化测试/test.html".into(),
            sha256: "1".repeat(64),
            size: 1,
            rows: 1,
            published: true,
        },
    )
    .unwrap();
    let done = app
        .finish_article_run_local(&f.account, &run.id, "partial", None)
        .unwrap();
    assert_eq!(done.run.state, "partial");
    app.article_job_action_local(
        &f.account,
        JobAction::Archive {
            id: run.id.clone(),
            revision: done.revision,
        },
    )
    .unwrap();
    assert!(
        app.article_run_local(&f.account, &run.id)
            .unwrap()
            .run
            .archived
    );
    assert!(
        app.article_plans_local(&f.account, None).unwrap().0[0]
            .plan
            .active_run
            .is_none()
    );
    let mut backup = vec![];
    assert_eq!(
        app.backup_library_local(&f.account, "owned-password", &key(), &mut backup)
            .unwrap(),
        1
    );
}
#[test]
fn article_cancellation_discards_uncommitted_network_result() {
    let f = Fixture::new();
    let id = key();
    f.reader.wait.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let request = id.clone();
    let worker = std::thread::spawn(move || {
        app.fetch_article_local(&account, "https://mp.weixin.qq.com/s/owned", &request)
    });
    f.app.cancel_browse_local(&id).unwrap();
    assert_eq!(worker.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
    assert!(
        f.app
            .articles_local(&f.account, None)
            .unwrap()
            .data
            .is_empty()
    );
}
#[tokio::test]
async fn article_http_and_mcp_enforce_read_write_and_account_scopes() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    let f = Fixture::new();
    f.app.enable_api(true);
    let token = f
        .app
        .issue_token_local(
            "owned article reader",
            vec![f.account.clone()],
            vec!["sources:read".into()],
            1,
        )
        .unwrap();
    let router = werc_http_api::router(f.app.clone());
    let send = |path: &str, body: Value| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("authorization", format!("Bearer {}", token.secret))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let resp = router
        .clone()
        .oneshot(send(
            "/api/v1/articles",
            json!({"account_id":f.account,"request_id":key(),"operation":{"action":"list"}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp=router.clone().oneshot(send("/api/v1/articles",json!({"account_id":f.account,"request_id":key(),"operation":{"action":"fetch","url":"https://mp.weixin.qq.com/s/owned"}}))).await.unwrap();
    assert_eq!(resp.status(), 403);
    let resp = router
        .clone()
        .oneshot(send(
            "/api/v1/articles/jobs",
            json!({"account_id":"another","operation":{"action":"plans"}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"api_read","arguments":{"method":"POST","path":"/api/v1/articles","body":{"account_id":f.account,"request_id":key(),"operation":{"action":"list"}}}}});
    let resp = router.oneshot(send("/mcp", body)).await.unwrap();
    assert_eq!(resp.status(), 200);
    let data: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(data.get("error").is_none(), "{data}");
    assert_ne!(data["result"]["isError"], true, "{data}");
}

#[test]
fn article_publisher_discovery_deduplicates_and_resume_checks_revisions(){
 let f=Fixture::new();let mut spec=spec();spec.publishers=vec!["gh_owned".into(),"gh_missing".into()];spec.formats.push("zip".into());
 let p=f.app.save_article_plan_local(&f.account,None,0,spec).unwrap();
 let run=f.app.begin_article_run_local(&f.account,&p.id,true).unwrap();
 let one=f.app.step_article_run_local(&f.account,&run.id).unwrap();assert_eq!(one.run.discovered_publishers,1);assert_eq!(one.run.spec.urls.len(),2);
 let two=f.app.step_article_run_local(&f.account,&run.id).unwrap();assert_eq!(two.run.discovery_errors.len(),1);
 let three=f.app.step_article_run_local(&f.account,&run.id).unwrap();let article=three.run.results[0].id.clone().unwrap();
 f.app.article_job_action_local(&f.account,JobAction::Cancel{id:run.id.clone()}).unwrap();
 f.app.confirm_article_local(&f.account,&article,1,true).unwrap();
 assert_eq!(f.app.article_job_action_local(&f.account,JobAction::Resume{id:run.id}).err().unwrap().code,"ARTICLE_RUN_CONTENT_CHANGED");
}
#[test]
fn article_empty_publisher_plan_reports_discovery_failure(){
 let f=Fixture::new();let mut spec=spec();spec.urls.clear();spec.publishers=vec!["gh_missing".into()];
 let p=f.app.save_article_plan_local(&f.account,None,0,spec).unwrap();let run=f.app.begin_article_run_local(&f.account,&p.id,true).unwrap();
 f.app.step_article_run_local(&f.account,&run.id).unwrap();
 let done=f.app.step_article_run_local(&f.account,&run.id).unwrap();assert_eq!(done.run.state,"failed");assert_eq!(done.run.discovery_errors.len(),1);assert!(f.app.article_plans_local(&f.account,None).unwrap().0[0].plan.active_run.is_none());
}
