use super::*;
use werc_application::live_moments::MomentExportFormat;
use werc_domain::{live::LiveCalendarDay, moments::MomentQuery};
#[test]
fn moments_tools_scan_sparse_filters_multisource_calendar_and_all_exports() {
    for multi in [false, true] {
        let f = Fixture::new();
        let view = if multi {
            let paths = two_sources(&f);
            f.app
                .prepare_live_filehelper_multi_local(
                    &f.account,
                    &paths,
                    "owned-moment-tools-parent",
                    true,
                )
                .unwrap()
                .view_id
        } else {
            f.prepare().unwrap().view_id
        };
        let q = MomentQuery {
            author: Some("owned-moment-1".into()),
            ..Default::default()
        };
        let page = f
            .app
            .query_live_moments_local(
                &f.account,
                &view,
                Some(q),
                None,
                "owned-moment-tools-filter",
            )
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.scanned, 31);
        assert!(page.next.is_none());
        assert_eq!(page.rows[0].anchor.row_id, "1");
        let query = MomentQuery::default();
        let days = vec![
            LiveCalendarDay {
                from_timestamp: 1699920000,
                until_timestamp: 1700000000,
            },
            LiveCalendarDay {
                from_timestamp: 1700000000,
                until_timestamp: 1700086400,
            },
        ];
        let overview = f
            .app
            .moment_overview_local(
                &f.account,
                &view,
                &query,
                &days,
                "owned-moment-tools-overview",
            )
            .unwrap();
        assert_eq!(overview.counts, vec![0, 31]);
        assert_eq!(overview.authors.len(), 31);
        assert_eq!(overview.scanned, 31);
        for format in [
            MomentExportFormat::Json,
            MomentExportFormat::Html,
            MomentExportFormat::Md,
            MomentExportFormat::Txt,
        ] {
            let export = f
                .app
                .export_live_moments_local(
                    &f.account,
                    &view,
                    &query,
                    format,
                    "owned-moment-tools-export",
                    Ok,
                )
                .unwrap();
            assert_eq!(export.summary.rows, 31);
            assert_eq!(export.summary.missing_media, 31);
            assert!(!export.summary.complete_timeline);
            assert!(String::from_utf8_lossy(&export.bytes).contains("owned-moment-1"));
            if matches!(format, MomentExportFormat::Json) {
                let data: serde_json::Value = serde_json::from_slice(&export.bytes).unwrap();
                assert_eq!(data["count"], 31);
                assert_eq!(data["rows"][0]["anchor"]["id"], (u64::MAX - 1).to_string());
            }
        }
        let query = MomentQuery {
            from: Some(1700000001),
            ..Default::default()
        };
        assert!(
            f.app
                .query_live_moments_local(
                    &f.account,
                    &view,
                    Some(query),
                    None,
                    "owned-moment-tools-empty"
                )
                .unwrap()
                .rows
                .is_empty()
        );
        f.app
            .cancel_browse_local("owned-moment-tools-cancel")
            .unwrap();
        let published = std::cell::Cell::new(false);
        assert!(
            f.app
                .export_live_moments_local(
                    &f.account,
                    &view,
                    &MomentQuery::default(),
                    MomentExportFormat::Json,
                    "owned-moment-tools-cancel",
                    |_| {
                        published.set(true);
                        Ok(())
                    }
                )
                .is_err()
        );
        assert!(!published.get());
    }
}
#[tokio::test]
async fn moments_tools_http_export_requires_export_scope() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    for permitted in [false, true] {
        let mut scopes = vec!["moments:read".into()];
        if permitted {
            scopes.push("exports:create".into());
        }
        let token = f
            .app
            .issue_token_local("owned moment export", vec![f.account.clone()], scopes, 1)
            .unwrap();
        let request=Request::builder().method("POST").uri("/api/v1/moments/live/export").header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json").body(Body::from(serde_json::json!({"account_id":f.account,"view_id":view.view_id,"query":{},"format":"json","request_id":"owned-moment-export-http"}).to_string())).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), if permitted { 200 } else { 403 });
        if permitted {
            assert_eq!(response.headers()["x-werc-records"], "31");
            let value: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(value["count"], 31);
        }
    }
}
