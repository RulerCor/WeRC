use super::*;
use sha2::Digest;
use werc_application::{capture_tools::*, library::LibraryEntry, source_capture::*};
fn complete(f: &Fixture) -> CaptureStatus {
    let parent = f.prepare().unwrap();
    let mut status = f
        .app
        .start_source_capture_local(&f.account, &parent.view_id, None, None)
        .unwrap();
    for _ in 0..30 {
        if status.complete_selected_sources {
            return status;
        }
        status = f
            .app
            .step_source_capture_local(&f.account, &status.id, "owned-capture-step")
            .unwrap();
    }
    panic!("capture did not progress");
}
#[test]
fn archive_name_snapshot_has_exact_ids_and_survives_encrypted_backup() {
    use std::io::{Cursor, Read};
    let f = Fixture::new();
    let status = complete(&f);
    let parent = f.prepare().unwrap();
    let names = f
        .app
        .refresh_capture_names_local(
            &f.account,
            &status.id,
            &parent.view_id,
            "owned-capture-directory",
        )
        .unwrap();
    assert_eq!(names.contacts, 201);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let patch = CapturePatch {
        message_type: "1".into(),
        timestamp: page.rows[0].message.timestamp,
        text: Some("自动化测试显示名".into()),
        extra: Some(CaptureFields {
            username: page.rows[0].username.clone(),
            sender_username: Some("owned-contact-201".into()),
            server_id: page.rows[0].message.server_id.clone(),
            metadata: None,
        }),
    };
    let edit = f
        .app
        .preview_capture_edit_local(&f.account, &status.id, 0, 0, page.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &edit.id, "apply")
        .unwrap();
    let mut options = CaptureExportOptions::default();
    options.display_name = "nickname".into();
    options.xlsx_columns = vec![
        "sender".into(),
        "nickname".into(),
        "remark".into(),
        "group_nickname".into(),
        "attachment".into(),
    ];
    let mut book = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "xlsx",
            &options,
            "owned-name-columns",
            &mut book,
        )
        .unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(book)).unwrap();
    let mut sheet = String::new();
    zip.by_name("xl/sharedStrings.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    assert!(sheet.contains("自有昵称") && sheet.contains("群内昵称"));
    options.redact_identifiers = true;
    let mut book = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "xlsx",
            &options,
            "owned-name-redaction",
            &mut book,
        )
        .unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(book)).unwrap();
    let mut sheet = String::new();
    zip.by_name("xl/sharedStrings.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    assert!(!sheet.contains("自有昵称") && !sheet.contains("owned-contact-201"));
    let mut backup = vec![];
    f.app
        .backup_library_local(
            &f.account,
            "owned-name-backup",
            "owned-name-backup-request",
            &mut backup,
        )
        .unwrap();
    let f2 = Fixture::new();
    f2.app
        .restore_library_local(
            &f2.account,
            "owned-name-backup",
            "owned-name-restore",
            &mut std::io::BufReader::new(backup.as_slice()),
        )
        .unwrap();
    let restored = f2.app.source_captures_local(&f2.account, None).unwrap();
    assert_eq!(
        f2.app
            .capture_names_status_local(&f2.account, &restored.data[0].id)
            .unwrap()
            .contacts,
        201
    );
}
#[test]
fn saved_transcripts_export_corrected_text_and_skip_changed_messages() {
    use std::io::{Cursor, Read};
    use werc_domain::asr::{AudioSource, Segment, Transcript};
    let f = Fixture::new();
    let status = complete(&f);
    let source = AudioSource::Archived {
        capture_id: status.id.clone(),
        page: 0,
        row: 0,
    };
    f.app
        .library_save_local(
            &f.account,
            None,
            "transcript",
            0,
            LibraryEntry {
                title: "自动化测试转写".into(),
                text: "人工校对后的测试文字".into(),
                tags: vec![],
                source: Some(serde_json::to_string(&source).unwrap()),
                transcript: Some(Transcript {
                    model: "owned".into(),
                    language: "zh".into(),
                    text: "人工校对后的测试文字".into(),
                    segments: vec![Segment {
                        from_ms: 0,
                        until_ms: 1000,
                        text: "人工校对后的测试文字".into(),
                    }],
                }),
            },
        )
        .unwrap();
    let mut options = CaptureExportOptions::default();
    options.xlsx_columns = vec!["transcript".into()];
    options.include_transcripts = true;
    let export = |options: &CaptureExportOptions, request: &str| {
        let mut book = vec![];
        f.app
            .export_capture_local(&f.account, &status.id, "xlsx", options, request, &mut book)
            .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(book)).unwrap();
        let mut strings = String::new();
        zip.by_name("xl/sharedStrings.xml")
            .unwrap()
            .read_to_string(&mut strings)
            .unwrap();
        strings
    };
    assert!(export(&options, "owned-transcript-export").contains("人工校对后的测试文字"));
    options.redact_identifiers = true;
    assert!(!export(&options, "owned-transcript-redaction").contains("人工校对后的测试文字"));
    options.redact_identifiers = false;
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let patch = CapturePatch {
        message_type: "1".into(),
        timestamp: page.rows[0].message.timestamp,
        text: Some("已修改本地副本".into()),
        extra: None,
    };
    let edit = f
        .app
        .preview_capture_edit_local(&f.account, &status.id, 0, 0, page.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &edit.id, "apply")
        .unwrap();
    assert!(!export(&options, "owned-edited-transcript-export").contains("人工校对后的测试文字"));
}
#[tokio::test]
async fn capture_export_names_and_transcripts_use_existing_read_scopes() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let f = Fixture::new();
    let status = complete(&f);
    f.app.enable_api(true);
    for (index, (options, extra, expected)) in [
        (serde_json::json!({"xlsx_columns":["text"]}), None, 200),
        (serde_json::json!({"xlsx_columns":["nickname"]}), None, 403),
        (
            serde_json::json!({"xlsx_columns":["nickname"]}),
            Some("contacts:read"),
            200,
        ),
        (
            serde_json::json!({"xlsx_columns":["sender"],"display_name":"remark"}),
            None,
            403,
        ),
        (
            serde_json::json!({"xlsx_columns":["transcript"],"include_transcripts":true}),
            None,
            403,
        ),
        (
            serde_json::json!({"xlsx_columns":["transcript"],"include_transcripts":true}),
            Some("sources:read"),
            200,
        ),
        (
            serde_json::json!({"redact_identifiers":true,"include_transcripts":true}),
            None,
            200,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut scopes = vec!["messages:read".into(), "exports:create".into()];
        if let Some(scope) = extra {
            scopes.push(scope.into());
        }
        let token = f
            .app
            .issue_token_local("owned options", vec![f.account.clone()], scopes, 1)
            .unwrap();
        let request=Request::builder().method("POST").uri("/api/v1/messages/recovery/export").header("authorization",format!("Bearer {}",token.secret)).header("content-type","application/json").body(Body::from(serde_json::json!({"account_id":f.account,"id":status.id,"format":"xlsx","options":options,"request_id":format!("owned-http-options-{index}")}).to_string())).unwrap();
        let response = werc_http_api::router(f.app.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected, "case {index}");
    }
}
#[test]
fn image_keys_only_save_after_complete_decode_and_stay_out_of_backups() {
    let f = Fixture::new();
    let png = include_bytes!("../../../../tests/fixtures/owned-image.png");
    let encrypted = png.iter().map(|b| b ^ 0x42).collect::<Vec<_>>();
    assert!(!f.app.image_key_status_local(&f.account).unwrap().saved);
    assert!(
        f.app
            .verify_image_keys_local(
                &f.account,
                png.to_vec(),
                None,
                Some(0x42),
                false,
                "owned-plain-not-key-proof"
            )
            .is_err()
    );
    assert!(
        f.app
            .verify_image_keys_local(
                &f.account,
                encrypted.clone(),
                None,
                Some(0x43),
                false,
                "owned-wrong-image-key"
            )
            .is_err()
    );
    let saved = f
        .app
        .verify_image_keys_local(
            &f.account,
            encrypted.clone(),
            None,
            None,
            true,
            "owned-image-key-recover",
        )
        .unwrap();
    assert!(saved.status.saved && saved.status.has_xor && !saved.status.has_aes);
    assert_eq!(
        f.app.reveal_image_keys_local(&f.account).unwrap().xor,
        Some(0x42)
    );
    let again = f
        .app
        .decode_cached_image_local(&f.account, encrypted.clone(), "owned-image-key-reopen")
        .unwrap();
    assert_eq!(saved.sha256, again.sha256);
    let mut backup = vec![];
    assert_eq!(
        f.app
            .backup_library_local(
                &f.account,
                "owned-key-backup",
                "owned-key-backup-request",
                &mut backup
            )
            .unwrap(),
        0
    );
    f.reader.context.lock().unwrap().sender_id = "other-login".into();
    assert!(
        f.app
            .decode_cached_image_local(&f.account, encrypted, "owned-image-key-other-login")
            .is_err()
    );
    f.app.remove_image_keys_local(&f.account).unwrap();
    assert!(!f.app.image_key_status_local(&f.account).unwrap().saved);
}

#[test]
fn interchange_formats_preserve_text_directions_and_custom_columns() {
    use std::io::{Cursor, Read};
    let f = Fixture::new();
    let status = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let mut options = CaptureExportOptions::default();
    options.filter.conversations = vec![page.rows[0].username.clone()];
    let mut json = vec![];
    let summary = f
        .app
        .export_capture_local(
            &f.account,
            &status.id,
            "chatlab_json",
            &options,
            "owned-chatlab-json",
            &mut json,
        )
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(doc["chatlab"]["version"], "0.0.2");
    assert!(summary.rows > 0);
    assert_eq!(
        doc["messages"].as_array().unwrap().len(),
        summary.rows as usize
    );
    let ids = doc["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["platformId"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    for m in doc["messages"].as_array().unwrap() {
        assert!(ids.contains(m["sender"].as_str().unwrap()));
        assert!(m["type"].is_number());
    }
    let mut lines = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "chatlab_jsonl",
            &options,
            "owned-chatlab-jsonl",
            &mut lines,
        )
        .unwrap();
    let lines = String::from_utf8(lines)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines[0]["_type"], "header");
    assert_eq!(
        lines.iter().filter(|v| v["_type"] == "message").count(),
        summary.rows as usize
    );
    options.xlsx_columns = vec!["time".into(), "sender".into(), "text".into()];
    let mut book = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "xlsx",
            &options,
            "owned-custom-xlsx",
            &mut book,
        )
        .unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(book)).unwrap();
    let mut sheet = String::new();
    zip.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet)
        .unwrap();
    assert!(sheet.contains("r=\"C1\"") && !sheet.contains("r=\"D1\""));
    assert!(!sheet.contains("<f>"));
    let mut script = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "sql",
            &options,
            "owned-sql-export",
            &mut script,
        )
        .unwrap();
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(std::str::from_utf8(&script).unwrap())
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM werc_messages", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        summary.rows
    );
    options.xlsx_columns.push("text".into());
    assert!(
        f.app
            .export_capture_local(
                &f.account,
                &status.id,
                "xlsx",
                &options,
                "owned-duplicate-xlsx-column",
                &mut Vec::new()
            )
            .is_err()
    );
    options.xlsx_columns.clear();
    options.filter.conversations.clear();
    assert!(
        f.app
            .export_capture_local(
                &f.account,
                &status.id,
                "chatlab_json",
                &options,
                "owned-chatlab-multiple",
                &mut Vec::new()
            )
            .is_err()
    );
}

#[test]
fn weclone_sender_direction_and_anonymous_group_keep_semantics() {
    let f = Fixture::new();
    let status = complete(&f);
    let mut options = CaptureExportOptions::default();
    let mut out = vec![];
    let summary = f
        .app
        .export_capture_local(
            &f.account,
            &status.id,
            "weclone_csv",
            &options,
            "owned-weclone-directions",
            &mut out,
        )
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.starts_with("id,MsgSvrID,type_name,is_sender,talker,room_name,msg,src,CreateTime")
    );
    assert_eq!(text.lines().count() as u64, summary.rows + 1);
    assert!(text.contains("\"0\",\"owned-sender-"));
    options.redact_identifiers = true;
    let mut out = vec![];
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "weclone_csv",
            &options,
            "owned-weclone-anonymous",
            &mut out,
        )
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("owned-sender-"));
    f.reader.legacy_sender.store(true, Ordering::SeqCst);
    let unknown = complete(&f);
    assert_eq!(
        f.app
            .export_capture_local(
                &f.account,
                &unknown.id,
                "weclone_csv",
                &CaptureExportOptions::default(),
                "owned-weclone-unknown",
                &mut Vec::new()
            )
            .err()
            .unwrap()
            .code,
        "EXPORT_SENDER_UNKNOWN"
    );
}
#[test]
fn chatlab_zip_selects_matching_saved_media_and_omits_binary_when_redacted() {
    use std::io::{Cursor, Read};
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = format!(
        "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabb\" length=\"100\"/></msg>",
        "11".repeat(16)
    );
    let status = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let saved = f
        .app
        .archive_capture_media_local(&f.account, &status.id, 0, 0, "owned-chatlab-save-media")
        .unwrap();
    let mut options = CaptureExportOptions::default();
    options.filter.conversations = vec![page.rows[0].username.clone()];
    let mut output = vec![];
    let summary = f
        .app
        .export_capture_local(
            &f.account,
            &status.id,
            "chatlab_zip",
            &options,
            "owned-chatlab-media-zip",
            &mut output,
        )
        .unwrap();
    assert_eq!(summary.media_written, 1);
    let mut zip = zip::ZipArchive::new(Cursor::new(output)).unwrap();
    let mut manifest = String::new();
    zip.by_name("attachments.json")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let entry = manifest["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["status"] == "saved")
        .unwrap();
    assert_eq!(entry["sha256"], saved.sha256);
    let path = entry["path"].as_str().unwrap();
    assert!(path.starts_with("media/") && !path.contains("..") && !path.contains(':'));
    let mut bytes = vec![];
    zip.by_name(path).unwrap().read_to_end(&mut bytes).unwrap();
    assert_eq!(hex::encode(sha2::Sha256::digest(bytes)), saved.sha256);
    let mut chat = String::new();
    zip.by_name("chatlab.json")
        .unwrap()
        .read_to_string(&mut chat)
        .unwrap();
    let chat: serde_json::Value = serde_json::from_str(&chat).unwrap();
    assert_eq!(
        chat["messages"][entry["messageIndex"].as_u64().unwrap() as usize]["type"],
        1
    );
    options.redact_identifiers = true;
    let mut output = vec![];
    let summary = f
        .app
        .export_capture_local(
            &f.account,
            &status.id,
            "chatlab_zip",
            &options,
            "owned-chatlab-redacted-zip",
            &mut output,
        )
        .unwrap();
    assert_eq!(summary.media_written, 0);
    let mut zip = zip::ZipArchive::new(Cursor::new(output)).unwrap();
    assert!(!(0..zip.len()).any(|i| zip.by_index(i).unwrap().name().starts_with("media/")));
    let patch = CapturePatch {
        message_type: "10000".into(),
        timestamp: page.rows[0].message.timestamp,
        text: Some("自动化测试系统行".into()),
        extra: None,
    };
    let edit = f
        .app
        .preview_capture_edit_local(&f.account, &status.id, 0, 0, page.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &edit.id, "apply")
        .unwrap();
    let updated = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let change = f
        .app
        .preview_capture_delete_local(&f.account, &status.id, 0, 0, updated.revision)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &change.id, "apply")
        .unwrap();
    let mut bundle = Cursor::new(Vec::new());
    let summary = f
        .app
        .export_capture_bundle_local(
            &f.account,
            &status.id,
            "owned-deleted-media-bundle",
            &mut bundle,
        )
        .unwrap();
    assert_eq!(summary.media, 0);
    let mut zip = zip::ZipArchive::new(Cursor::new(bundle.into_inner())).unwrap();
    assert!(!(0..zip.len()).any(|i| zip.by_index(i).unwrap().name() == "media/0-0.png"));
}
#[tokio::test]
async fn capture_export_http_preserves_scope_and_media_scope_boundaries() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    let status = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    f.app.enable_api(true);
    for (format, extra, expected) in [
        ("json", false, 200),
        ("chatlab_zip", false, 403),
        ("chatlab_zip", true, 200),
    ] {
        let mut scopes = vec!["messages:read".into(), "exports:create".into()];
        if extra {
            scopes.push("media:read".into());
        }
        let token = f
            .app
            .issue_token_local("owned export", vec![f.account.clone()], scopes, 1)
            .unwrap();
        let request=Request::builder().method("POST").uri("/api/v1/messages/recovery/export").header("authorization",format!("Bearer {}",token.secret)).header("content-type","application/json").body(Body::from(serde_json::json!({"account_id":f.account,"id":status.id,"format":format,"options":{"filter":{"conversations":[page.rows[0].username]}},"request_id":format!("owned-http-export-{format}-{extra}").replace('_',"-")}).to_string())).unwrap();
        let response = werc_http_api::router(f.app.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        if expected == 200 {
            assert_eq!(response.headers()["cache-control"], "no-store");
            let bytes = to_bytes(response.into_body(), 64 * 1024 * 1024)
                .await
                .unwrap();
            assert!(!bytes.is_empty());
        }
    }
}

#[test]
fn export_message_kind_keyword_and_xml_subtype_survive_public_projection() {
    let f = Fixture::new();
    let status = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let patch=CapturePatch{message_type:"49".into(),timestamp:page.rows[0].message.timestamp,text:Some("Owned 自动化测试链接".into()),extra:Some(CaptureFields{username:page.rows[0].username.clone(),sender_username:page.rows[0].message.sender_username.clone(),server_id:page.rows[0].message.server_id.clone(),metadata:Some("<msg><appmsg><type>5</type><title>自动化测试</title><url>https://example.test/</url><appattach><aeskey>private-not-exported</aeskey></appattach></appmsg></msg>".into())})};
    let edit = f
        .app
        .preview_capture_edit_local(&f.account, &status.id, 0, 0, page.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &edit.id, "apply")
        .unwrap();
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    assert_eq!(page.rows[0].app_subtype, Some(5));
    assert!(page.rows[0].message.metadata.is_none());
    let mut options = CaptureExportOptions::default();
    options.filter.message_kinds = vec!["link".into()];
    options.filter.keyword = "OWNED 自动化".into();
    let mut bytes = vec![];
    let summary = f
        .app
        .export_capture_local(
            &f.account,
            &status.id,
            "chatlab_json",
            &options,
            "owned-export-filtered-kind",
            &mut bytes,
        )
        .unwrap();
    assert_eq!(summary.rows, 1);
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(doc["messages"][0]["type"], 7);
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("private-not-exported")
    );
    options.filter.message_kinds = vec!["voice".into()];
    let report = f
        .app
        .capture_report_local(
            &f.account,
            &status.id,
            &options.filter,
            "owned-report-filtered-kind",
        )
        .unwrap();
    assert_eq!(report.counted_rows, 0);
    options.filter.message_kinds = vec!["made-up".into()];
    assert!(
        f.app
            .capture_report_local(
                &f.account,
                &status.id,
                &options.filter,
                "owned-filter-invalid-kind"
            )
            .is_err()
    );
}

fn plan_spec(capture: &str) -> werc_application::export_plans::ExportPlanSpec {
    werc_application::export_plans::ExportPlanSpec {
        title: "自动化测试归档".into(),
        source_capture: capture.into(),
        refresh_source: true,
        formats: vec!["html".into(), "jsonl".into()],
        directory: "WeRC-自动化测试".into(),
        options: CaptureExportOptions::default(),
        interval_seconds: 3600,
        lookback_seconds: Some(86400),
        since_last_success: true,
        enabled: true,
        next_run: 0,
        stop_at: None,
        max_runs: Some(2),
    }
}
#[test]
fn export_plan_failure_cancel_resume_and_success_watermark_are_durable() {
    use werc_application::export_plans::*;
    let f = Fixture::new();
    let capture = complete(&f);
    let plan = f
        .app
        .save_export_plan_local(&f.account, None, 0, plan_spec(&capture.id))
        .unwrap();
    let first = f
        .app
        .begin_export_run_local(&f.account, &plan.id, false)
        .unwrap();
    assert!(
        f.app
            .begin_export_run_local(&f.account, &plan.id, true)
            .is_err()
    );
    assert!(
        f.app
            .save_export_plan_local(
                &f.account,
                Some(&plan.id),
                plan.revision,
                plan_spec(&capture.id)
            )
            .is_err()
    );
    f.app
        .finish_export_run_local(
            &f.account,
            &first.id,
            "failed",
            Some(werc_domain::AppError::new("OWNED", "自动化测试故障")),
        )
        .unwrap();
    let after = f
        .app
        .export_plans_local(&f.account, None)
        .unwrap()
        .0
        .remove(0);
    assert_eq!(after.plan.last_success_until, None);
    assert_eq!(after.plan.success_count, 0);
    let app = Application::new(f.repo.clone(), f.reader.clone());
    app.resume_export_run_local(&f.account, &first.id).unwrap();
    app.cancel_export_run_local(&f.account, &first.id).unwrap();
    assert!(
        app.export_run_phase_local(&f.account, &first.id, "reading")
            .is_err()
    );
    app.finish_export_run_local(&f.account, &first.id, "canceled", None)
        .unwrap();
    app.resume_export_run_local(&f.account, &first.id).unwrap();
    app.export_run_phase_local(&f.account, &first.id, "exporting")
        .unwrap();
    assert_eq!(
        app.finish_export_run_local(&f.account, &first.id, "succeeded", None)
            .err()
            .unwrap()
            .code,
        "EXPORT_INCOMPLETE"
    );
    for format in ["html", "jsonl"] {
        app.record_export_artifact_local(
            &f.account,
            &first.id,
            ExportArtifact {
                format: format.into(),
                path: format!("owned.{format}"),
                sha256: "a".repeat(64),
                size: 10,
                rows: 500,
                published: true,
            },
        )
        .unwrap();
    }
    app.finish_export_run_local(&f.account, &first.id, "succeeded", None)
        .unwrap();
    let after = app
        .export_plans_local(&f.account, None)
        .unwrap()
        .0
        .remove(0);
    assert_eq!(after.plan.success_count, 1);
    assert_eq!(after.plan.run_count, 1);
    assert_eq!(after.plan.last_success_until, Some(first.run.until));
    let second = app
        .begin_export_run_local(&f.account, &plan.id, true)
        .unwrap();
    assert_eq!(second.run.from, Some(first.run.until));
    if second.run.until == first.run.until {
        assert_eq!(second.run.state, "no_new_data");
        assert!(second.run.ended_at.is_some());
    }

    app.finish_export_run_local(&f.account, &second.id, "failed", None)
        .unwrap();
    assert_eq!(
        app.begin_export_run_local(&f.account, &plan.id, false)
            .err()
            .unwrap()
            .code,
        "NOT_DUE"
    );
    let after = app
        .export_plans_local(&f.account, None)
        .unwrap()
        .0
        .remove(0);
    app.delete_export_plan_local(&f.account, &plan.id, after.revision)
        .unwrap();
    assert_eq!(app.export_runs_local(&f.account, None).unwrap().0.len(), 2);
    assert!(app.resume_export_run_local(&f.account, &second.id).is_err());
}
#[test]
fn export_plan_validates_paths_ranges_and_capture_forks_resume_without_parent_view() {
    let f = Fixture::new();
    let capture = complete(&f);
    for directory in ["../escape", "C:\\developer\\exports", "NUL:stream"] {
        let mut spec = plan_spec(&capture.id);
        spec.directory = directory.into();
        assert!(
            f.app
                .save_export_plan_local(&f.account, None, 0, spec)
                .is_err()
        );
    }
    let mut spec = plan_spec(&capture.id);
    spec.interval_seconds = 0;
    assert!(
        f.app
            .save_export_plan_local(&f.account, None, 0, spec)
            .is_err()
    );
    assert!(
        f.app
            .save_export_plan_local(&f.account, None, i64::MAX, plan_spec(&capture.id))
            .is_err()
    );
    let until = chrono::Utc::now().timestamp();
    let fork = f
        .app
        .fork_source_capture_local(&f.account, &capture.id, "owned-job-capture", None, until)
        .unwrap();
    let progressed = f
        .app
        .step_source_capture_local(&f.account, &fork.id, "owned-fork-capture")
        .unwrap();
    let resumed = f
        .app
        .fork_source_capture_local(&f.account, &capture.id, &fork.id, None, until)
        .unwrap();
    assert_eq!(resumed.pages, progressed.pages);
    assert!(
        f.app
            .source_capture_status_local(&f.account, &capture.id)
            .unwrap()
            .complete_selected_sources
    );
}
#[test]
fn capture_reports_preserve_counting_and_escape_exported_content() {
    let f = Fixture::new();
    let status = complete(&f);
    let report = f
        .app
        .capture_report_local(
            &f.account,
            &status.id,
            &CaptureFilter::default(),
            "owned-rich-report",
        )
        .unwrap();
    assert_eq!(
        report.sent + report.received + report.unknown_sender,
        report.counted_rows
    );
    assert_eq!(
        report.conversations.values().map(|v| v.total).sum::<u64>(),
        500
    );
    assert!(report.active_days > 0);
    assert!(report.longest_streak > 0);
    let mut filter = CaptureFilter::default();
    filter.exclude_conversations = report.by_conversation.keys().cloned().collect();
    assert_eq!(
        f.app
            .capture_report_local(&f.account, &status.id, &filter, "owned-empty-report")
            .unwrap()
            .counted_rows,
        0
    );
    let mut report = report;
    report.common_phrases = vec![("<img onerror=alert(1)>".into(), 2)];
    let html = String::from_utf8(report.export("html").unwrap()).unwrap();
    assert!(!html.contains("<img"));
    assert!(html.contains("&lt;img"));
    assert!(report.export("exe").is_err());
}

#[test]
fn report_sections_keep_monthly_contacts_phrases_and_group_identity_distinct() {
    let f = Fixture::new();
    let capture = complete(&f);
    let login = f.reader.context.lock().unwrap().sender_id.clone();
    let peer = "owned-report-peer";
    let samples = [
        (peer, Some(login.as_str()), "共同短句"),
        (peer, Some(peer), "共同短句"),
        (peer, Some(login.as_str()), "我的专属"),
        (peer, Some(login.as_str()), "我的专属"),
        (peer, Some(peer), "对方专属"),
        (peer, Some(peer), "对方专属"),
        ("owned@im.chatroom", Some(peer), "群内文字"),
        ("gh_owned", Some("gh_owned"), "公众号文字"),
        (peer, None, "共同短句"),
    ];
    for (index, (username, sender, text)) in samples.into_iter().enumerate() {
        let page = f
            .app
            .source_capture_page_local(&f.account, &capture.id, 0)
            .unwrap();
        let mut patch = f
            .app
            .capture_edit_fields_local(&f.account, &capture.id, 0, index)
            .unwrap();
        patch.message_type = "1".into();
        patch.text = Some(text.into());
        patch.timestamp = 1_700_000_000 + index as i64 * 60;
        let fields = patch.extra.as_mut().unwrap();
        fields.username = username.into();
        fields.sender_username = sender.map(str::to_owned);
        fields.metadata = None;
        let change = f
            .app
            .preview_capture_edit_local(&f.account, &capture.id, 0, index, page.revision, patch)
            .unwrap();
        f.app
            .apply_capture_edit_local(&f.account, &change.id, "apply")
            .unwrap();
    }
    let filter = CaptureFilter {
        conversations: vec![peer.into()],
        view: AnalysisView::Private,
        ..Default::default()
    };
    let report = f
        .app
        .capture_report_local(&f.account, &capture.id, &filter, "owned-report-sections")
        .unwrap();
    assert_eq!(report.counted_rows, 7);
    assert_eq!(
        (report.sent, report.received, report.unknown_sender),
        (3, 3, 1)
    );
    assert_eq!(
        report.phrase_comparison.shared,
        vec![("共同短句".into(), 1, 1)]
    );
    assert_eq!(
        report.phrase_comparison.mine_only,
        vec![("我的专属".into(), 2)]
    );
    assert_eq!(
        report.phrase_comparison.others_only,
        vec![("对方专属".into(), 2)]
    );
    assert_eq!(
        report.monthly_leaders.values().next().unwrap()[0].username,
        peer
    );
    assert_eq!(
        report.monthly_leaders.values().next().unwrap()[0].messages,
        7
    );
    assert_eq!(report.conversations[peer].by_sender[&login], 3);
    assert_eq!(report.conversations[peer].by_type["1"], 7);
    assert_eq!(
        report.conversations[peer]
            .first_record
            .as_ref()
            .unwrap()
            .row,
        0
    );
    assert_eq!(
        report.conversations[peer].last_record.as_ref().unwrap().row,
        8
    );
    assert_eq!(
        report.conversations[peer]
            .first_record
            .as_ref()
            .unwrap()
            .content_status,
        "edited_local"
    );
    assert_eq!(report.conversations[peer].peak_day.as_ref().unwrap().1, 7);
    assert!(report.filter == filter);
    assert!(chrono::DateTime::parse_from_rfc3339(&report.generated_at).is_ok());
    let group_filter = CaptureFilter {
        conversations: vec!["owned@im.chatroom".into()],
        view: AnalysisView::Group,
        ..Default::default()
    };
    let group = f
        .app
        .capture_report_local(
            &f.account,
            &capture.id,
            &group_filter,
            "owned-group-report-sections",
        )
        .unwrap();
    assert_eq!(group.received, 1);
    assert_eq!(group.conversations["owned@im.chatroom"].by_sender[peer], 1);
    assert!(group.monthly_leaders.is_empty());
    assert_eq!(group.conversations["owned@im.chatroom"].their_turns, 0);
    let text = String::from_utf8(report.export("txt").unwrap()).unwrap();
    assert!(
        text.contains("每月主要联系人") && text.contains("共同短句") && text.contains("我的专属")
    );
}
#[test]
fn capture_all_conversations_survives_connection_epoch_and_application_restart() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let mut status = f
        .app
        .start_source_capture_local(&f.account, &parent.view_id, None, None)
        .unwrap();
    status = f
        .app
        .step_source_capture_local(&f.account, &status.id, "owned-capture-step")
        .unwrap();
    assert_eq!(status.rows, 60);
    assert_eq!(status.pages, 1);
    f.reader.context.lock().unwrap().epoch = "owned-reconnected-epoch".into();
    let app = Application::new(f.repo.clone(), f.reader.clone());
    for _ in 0..30 {
        if status.complete_selected_sources {
            break;
        }
        status = app
            .step_source_capture_local(&f.account, &status.id, "owned-capture-step")
            .unwrap();
    }
    assert!(status.complete_selected_sources);
    assert_eq!(status.rows, 500);
    assert!(!status.native_gaps_closed);
    let report = app
        .capture_report_local(
            &f.account,
            &status.id,
            &CaptureFilter::default(),
            "owned-capture-report",
        )
        .unwrap();
    assert_eq!(report.counted_rows, 500);
    assert_eq!(report.by_conversation.len(), 2);
    let protected = f
        .repo
        .library_get(&f.account, &format!("{}:0", status.id))
        .unwrap();
    assert!(!String::from_utf8_lossy(&protected.ciphertext).contains("owned-message"));
}
#[test]
fn capture_cancellation_and_source_replacement_never_advance_checkpoint() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let status = f
        .app
        .start_source_capture_local(&f.account, &parent.view_id, None, None)
        .unwrap();
    f.app.cancel_browse_local("owned-capture-cancel").unwrap();
    assert_eq!(
        f.app
            .step_source_capture_local(&f.account, &status.id, "owned-capture-cancel")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(
        f.app
            .source_capture_status_local(&f.account, &status.id)
            .unwrap()
            .pages,
        0
    );
    f.reader.context.lock().unwrap().sender_id = "other-login".into();
    assert!(
        f.app
            .step_source_capture_local(&f.account, &status.id, "owned-capture-other")
            .is_err()
    );
    assert_eq!(
        f.app
            .source_capture_status_local(&f.account, &status.id)
            .unwrap()
            .pages,
        0
    );
}
#[test]
fn capture_local_edit_preview_apply_conflict_and_restore_keep_original() {
    let f = Fixture::new();
    let status = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let original = page.rows[0].message.text.clone();
    let change = f
        .app
        .preview_capture_edit_local(
            &f.account,
            &status.id,
            0,
            0,
            page.revision,
            CapturePatch {
                extra: None,
                text: Some("owned edited <script> {{date}}".into()),
                timestamp: page.rows[0].message.timestamp,
                message_type: "1".into(),
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .source_capture_page_local(&f.account, &status.id, 0)
            .unwrap()
            .rows[0]
            .message
            .text,
        original
    );
    f.app
        .apply_capture_edit_local(&f.account, &change.id, "apply")
        .unwrap();
    assert_eq!(
        f.app
            .source_capture_page_local(&f.account, &status.id, 0)
            .unwrap()
            .rows[0]
            .message
            .content_status,
        "edited_local"
    );
    assert!(
        f.app
            .apply_capture_edit_local(&f.account, &change.id, "apply")
            .is_err()
    );
    let mut html = Vec::new();
    f.app
        .export_capture_local(
            &f.account,
            &status.id,
            "html",
            &CaptureExportOptions::default(),
            "owned-capture-export",
            &mut html,
        )
        .unwrap();
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
    f.app
        .apply_capture_edit_local(&f.account, &change.id, "restore")
        .unwrap();
    assert_eq!(
        f.app
            .source_capture_page_local(&f.account, &status.id, 0)
            .unwrap()
            .rows[0]
            .message
            .text,
        original
    );
}
#[test]
fn portable_library_backup_roundtrip_and_truncation_are_atomic() {
    let f = Fixture::new();
    let status = complete(&f);
    f.app
        .library_save_local(
            &f.account,
            None,
            "note",
            0,
            LibraryEntry {
                transcript: None,
                title: "owned private title".into(),
                text: "owned private content".into(),
                tags: vec![],
                source: None,
            },
        )
        .unwrap();
    let saved = f
        .app
        .library_save_local(
            &f.account,
            None,
            "transcript",
            0,
            LibraryEntry {
                title: "自动化测试字幕".into(),
                text: "owned subtitle".into(),
                tags: vec![],
                source: None,
                transcript: Some(werc_domain::asr::Transcript {
                    model: "owned".into(),
                    language: "en".into(),
                    text: "owned subtitle".into(),
                    segments: vec![werc_domain::asr::Segment {
                        from_ms: 120,
                        until_ms: 1500,
                        text: "owned subtitle".into(),
                    }],
                }),
            },
        )
        .unwrap();
    let page = f
        .app
        .source_capture_page_local(&f.account, &status.id, 0)
        .unwrap();
    let change = f
        .app
        .preview_capture_edit_local(
            &f.account,
            &status.id,
            0,
            0,
            page.revision,
            CapturePatch {
                extra: None,
                text: Some("owned corrected".into()),
                timestamp: page.rows[0].message.timestamp,
                message_type: "1".into(),
            },
        )
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &change.id, "apply")
        .unwrap();
    f.app
        .save_export_plan_local(&f.account, None, 0, plan_spec(&status.id))
        .unwrap();
    let mut bytes = Vec::new();
    let count = f
        .app
        .backup_library_local(
            &f.account,
            "owned backup password",
            "owned-backup-create",
            &mut bytes,
        )
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("owned private"));
    let original = f
        .repo
        .library_list(&f.account, "*", None, 100)
        .unwrap()
        .data
        .len();
    let mut bad = std::io::Cursor::new(&bytes[..bytes.len() - 30]);
    assert!(
        f.app
            .restore_library_local(
                &f.account,
                "owned backup password",
                "owned-backup-truncated",
                &mut bad
            )
            .is_err()
    );
    assert_eq!(
        f.repo
            .library_list(&f.account, "*", None, 100)
            .unwrap()
            .data
            .len(),
        original
    );
    assert!(
        f.app
            .restore_library_local(
                &f.account,
                "incorrect password",
                "owned-backup-password",
                &mut std::io::Cursor::new(&bytes)
            )
            .is_err()
    );
    assert_eq!(
        f.app
            .restore_library_local(
                &f.account,
                "owned backup password",
                "owned-backup-restore",
                &mut std::io::Cursor::new(&bytes)
            )
            .unwrap(),
        count
    );
    let captures = f.app.source_captures_local(&f.account, None).unwrap();
    assert_eq!(captures.data.len(), 2);
    let imported = captures.data.iter().find(|c| c.id != status.id).unwrap();
    let changes = f.app.capture_edits_local(&f.account, None).unwrap().0;
    assert!(
        changes
            .iter()
            .any(|c| c.state == "archived" && c.capture_id == imported.id)
    );
    assert_eq!(
        f.app.export_plans_local(&f.account, None).unwrap().0.len(),
        1
    );
    let transcripts = f
        .app
        .library_page_local(&f.account, "transcript", None)
        .unwrap();
    let transcript = transcripts
        .data
        .iter()
        .find(|r| r.header.id != saved.header.id)
        .unwrap()
        .entry
        .transcript
        .as_ref()
        .unwrap();
    assert_eq!(transcript.segments[0].until_ms, 1500);
    assert_eq!(imported.rows, status.rows);
    assert_eq!(
        f.app
            .source_capture_page_local(&f.account, &imported.id, 0)
            .unwrap()
            .rows
            .len(),
        60
    );
    let mut again = Vec::new();
    f.app
        .backup_library_local(
            &f.account,
            "owned backup password",
            "owned-backup-second",
            &mut again,
        )
        .unwrap();
    f.app
        .restore_library_local(
            &f.account,
            "owned backup password",
            "owned-backup-reimport",
            &mut std::io::Cursor::new(&again),
        )
        .unwrap();
}
#[tokio::test]
async fn mcp_full_api_preserves_read_endpoint_and_per_route_permissions() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    f.app.enable_api(true);
    let token = f
        .app
        .issue_token_local(
            "owned read",
            vec![f.account.clone()],
            vec!["accounts:read".into(), "messages:read".into()],
            1,
        )
        .unwrap();
    for (endpoint, tool, method, path, expect_error) in [
        ("/mcp/read", "api_read", "GET", "/api/v1/accounts", false),
        (
            "/mcp/read",
            "api_read",
            "POST",
            "/api/v1/messages/send-text",
            true,
        ),
        (
            "/mcp",
            "api_call",
            "POST",
            "/api/v1/messages/send-text",
            true,
        ),
    ] {
        let response=werc_http_api::router(f.app.clone()).oneshot(Request::builder().method("POST").uri(endpoint).header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json").body(Body::from(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":{"method":method,"path":path,"idempotency_key":"owned-no-send-0001","body":{"account_id":f.account,"username":"filehelper","text":"owned automation","wait_seconds":0}}}}).to_string())).unwrap()).await.unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value["result"]["isError"], expect_error, "{value}");
    }
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn durable_capture_media_survives_reopen_backup_and_offline_bundle() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = format!(
        "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabb\" length=\"100\"/></msg>",
        "11".repeat(16)
    );
    let capture = complete(&f);
    let media = f
        .app
        .archive_capture_media_local(&f.account, &capture.id, 0, 0, "owned-media-save")
        .unwrap();
    assert_eq!(media.mime, "image/png");
    assert!(
        f.repo.files(&f.account).unwrap().is_empty(),
        "archive temporary file must be released"
    );
    let app = Application::new(f.repo.clone(), f.reader.clone());
    let mut bytes = Vec::new();
    let reopened = app
        .export_capture_media_local(
            &f.account,
            &capture.id,
            0,
            0,
            "owned-media-open",
            &mut bytes,
        )
        .unwrap();
    assert_eq!(media.sha256, reopened.sha256);
    assert_eq!(hex::encode(sha2::Sha256::digest(&bytes)), media.sha256);
    assert_eq!(
        app.archive_capture_media_local(&f.account, &capture.id, 0, 0, "owned-media-reuse")
            .unwrap()
            .blob_id,
        media.blob_id
    );
    let page = app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let patch = CapturePatch {
        extra: None,
        text: Some("<script>unsafe</script>".into()),
        timestamp: page.rows[0].message.timestamp,
        message_type: "1".into(),
    };
    let change = app
        .preview_capture_edit_local(&f.account, &capture.id, 0, 0, page.revision, patch)
        .unwrap();
    app.apply_capture_edit_local(&f.account, &change.id, "apply")
        .unwrap();
    let list = app
        .capture_media_page_local(&f.account, &capture.id, 0)
        .unwrap();
    assert!(!list[0].matches_message);
    assert!(!list[0].can_fetch);
    let mut output = std::io::Cursor::new(Vec::new());
    let summary = app
        .export_capture_bundle_local(&f.account, &capture.id, "owned-media-bundle", &mut output)
        .unwrap();
    assert_eq!(summary.media, 1);
    assert_eq!(summary.rows, capture.rows);
    assert!(summary.missing_media > 0);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(output.into_inner())).unwrap();
    let mut html = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("index.html").unwrap(), &mut html).unwrap();
    assert!(html.contains("&lt;script&gt;unsafe&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(html.contains("media/0-0.png"));
    assert!(!html.contains("cdnmidimgurl"));
    let mut archived = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("media/0-0.png").unwrap(), &mut archived).unwrap();
    assert_eq!(archived, bytes);
    let mut backup = Vec::new();
    app.backup_library_local(
        &f.account,
        "owned media password",
        "owned-media-backup",
        &mut backup,
    )
    .unwrap();
    app.restore_library_local(
        &f.account,
        "owned media password",
        "owned-media-restore",
        &mut std::io::Cursor::new(backup),
    )
    .unwrap();
    let restored = app
        .source_captures_local(&f.account, None)
        .unwrap()
        .data
        .into_iter()
        .find(|c| c.imported)
        .unwrap();
    let mut restored_bytes = Vec::new();
    app.export_capture_media_local(
        &f.account,
        &restored.id,
        0,
        0,
        "owned-media-restored",
        &mut restored_bytes,
    )
    .unwrap();
    assert_eq!(restored_bytes, bytes);
    assert_eq!(
        app.archive_capture_media_local(&f.account, &restored.id, 0, 1, "owned-media-forbidden")
            .err()
            .unwrap()
            .code,
        "ARCHIVE_READ_ONLY"
    );
    let progress = app
        .step_capture_media_local(&f.account, &restored.id, false, "owned-media-progress")
        .unwrap();
    assert_eq!(progress.saved, 1);
    let app = Application::new(f.repo.clone(), f.reader.clone());
    assert_eq!(
        app.capture_media_progress_local(&f.account, &restored.id)
            .unwrap()
            .row,
        1
    );
}

#[test]
fn archived_voice_batch_transcripts_resume_and_reuse_corrected_text() {
    let f = Fixture::new();
    f.reader.voice.store(true, Ordering::SeqCst);
    let capture = complete(&f);
    f.app
        .archive_capture_media_local(&f.account, &capture.id, 0, 0, "owned-voice-archive")
        .unwrap();
    let p = f
        .app
        .step_capture_transcripts_local(
            &f.account,
            &capture.id,
            "owned",
            "auto",
            false,
            "owned-asr-step-0001",
        )
        .unwrap();
    assert_eq!(p.saved, 1);
    assert_eq!(p.row, 1);
    let mut item = f
        .app
        .library_page_local(&f.account, "transcript", None)
        .unwrap()
        .data
        .remove(0);
    item.entry.text = "自动化测试已校对".into();
    let transcript = item.entry.transcript.as_mut().unwrap();
    transcript.text = item.entry.text.clone();
    transcript.segments[0].text = item.entry.text.clone();
    f.app
        .library_save_local(
            &f.account,
            Some(&item.header.id),
            "transcript",
            item.header.revision,
            item.entry,
        )
        .unwrap();
    let reads = f.reader.reads.load(Ordering::SeqCst);
    let app = Application::new(f.repo.clone(), f.reader.clone());
    assert_eq!(
        app.capture_transcript_progress_local(&f.account, &capture.id, "owned", "auto")
            .unwrap()
            .row,
        1
    );
    let p = app
        .step_capture_transcripts_local(
            &f.account,
            &capture.id,
            "owned",
            "auto",
            true,
            "owned-asr-restart",
        )
        .unwrap();
    assert_eq!(p.saved, 1);
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), reads);
    let output = app
        .transcribe_local(
            &f.account,
            &werc_domain::asr::AudioSource::Archived {
                capture_id: capture.id.clone(),
                page: 0,
                row: 0,
            },
            "owned",
            "auto",
            "owned-asr-cache-0001",
        )
        .unwrap();
    assert_eq!(output.text, "自动化测试已校对");
    let mut archive = std::io::Cursor::new(Vec::new());
    let summary = app
        .export_capture_bundle_local(
            &f.account,
            &capture.id,
            "owned-subtitle-bundle",
            &mut archive,
        )
        .unwrap();
    assert_eq!(summary.transcripts, 1);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive.into_inner())).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("transcripts/0.txt").unwrap(), &mut text)
        .unwrap();
    assert_eq!(text, output.text);
}

#[test]
fn offline_fields_add_delete_restore_are_atomic_and_exports_mark_changes() {
    let f = Fixture::new();
    let capture = complete(&f);
    let before = f
        .app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let mut patch = f
        .app
        .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
        .unwrap();
    patch.message_type = "10000".into();
    patch.extra.as_mut().unwrap().metadata = Some("<msg><sysmsg>自动化测试</sysmsg></msg>".into());
    let change = f
        .app
        .preview_capture_edit_local(&f.account, &capture.id, 0, 0, before.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &change.id, "apply")
        .unwrap();
    assert!(
        f.app
            .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
            .unwrap()
            .extra
            .unwrap()
            .metadata
            .unwrap()
            .contains("自动化测试")
    );
    let page = f
        .app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let del = f
        .app
        .preview_capture_delete_local(&f.account, &capture.id, 0, 0, page.revision)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &del.id, "apply")
        .unwrap();
    let report = f
        .app
        .capture_report_local(
            &f.account,
            &capture.id,
            &CaptureFilter::default(),
            "owned-delete-report",
        )
        .unwrap();
    assert_eq!(report.counted_rows, capture.rows - 1);
    f.app
        .apply_capture_edit_local(&f.account, &del.id, "restore")
        .unwrap();
    let add = f
        .app
        .preview_capture_add_local(
            &f.account,
            &capture.id,
            "filehelper",
            "自动化测试本地新增",
            1700000000,
        )
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &add.id, "apply")
        .unwrap();
    let status = f
        .app
        .source_capture_status_local(&f.account, &capture.id)
        .unwrap();
    assert_eq!(status.rows, capture.rows + 1);
    assert_eq!(status.pages, capture.pages + 1);
    let page = f
        .app
        .source_capture_page_local(&f.account, &capture.id, capture.pages)
        .unwrap();
    assert_eq!(page.rows[0].message.content_status, "added_local");
    assert!(page.rows[0].message.server_id.is_none());
    for format in ["html", "txt", "md", "csv"] {
        let mut bytes = Vec::new();
        f.app
            .export_capture_local(
                &f.account,
                &capture.id,
                format,
                &CaptureExportOptions::default(),
                "owned-edited-export",
                &mut bytes,
            )
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(if format == "csv" {
            "added_local"
        } else {
            "本地新增记录"
        }));
        assert!(text.contains(if format == "csv" {
            "edited_local"
        } else {
            "本地已修改"
        }));
    }
    let reopened = Application::new(f.repo.clone(), f.reader.clone());
    reopened
        .apply_capture_edit_local(&f.account, &add.id, "restore")
        .unwrap();
    assert_eq!(
        reopened
            .source_capture_page_local(&f.account, &capture.id, capture.pages)
            .unwrap()
            .rows[0]
            .message
            .content_status,
        "deleted_local"
    );
    assert_eq!(
        reopened
            .capture_report_local(
                &f.account,
                &capture.id,
                &CaptureFilter::default(),
                "owned-added-report"
            )
            .unwrap()
            .counted_rows,
        capture.rows
    );
}

#[test]
fn archive_catalog_and_original_restore_survive_later_edits() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = format!(
        "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabb\" length=\"100\"/></msg>",
        "11".repeat(16)
    );
    let capture = complete(&f);
    let m = f
        .app
        .archive_capture_media_local(&f.account, &capture.id, 0, 0, "owned-catalog-save")
        .unwrap();
    let query = MediaQuery {
        saved: Some(true),
        ..Default::default()
    };
    let list = f
        .app
        .capture_media_catalog_local(&f.account, &capture.id, &query, None, "owned-catalog-list")
        .unwrap();
    assert_eq!(list.data.len(), 1);
    assert_eq!(list.data[0].media.saved.as_ref().unwrap().sha256, m.sha256);
    let before = f
        .app
        .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
        .unwrap();
    for text in ["自动化测试第一改", "自动化测试第二改"] {
        let page = f
            .app
            .source_capture_page_local(&f.account, &capture.id, 0)
            .unwrap();
        let mut patch = f
            .app
            .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
            .unwrap();
        patch.text = Some(text.into());
        patch.message_type = "1".into();
        let c = f
            .app
            .preview_capture_edit_local(&f.account, &capture.id, 0, 0, page.revision, patch)
            .unwrap();
        f.app
            .apply_capture_edit_local(&f.account, &c.id, "apply")
            .unwrap();
    }
    let page = f
        .app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let restore = f
        .app
        .preview_capture_original_local(&f.account, &capture.id, 0, 0, page.revision)
        .unwrap();
    assert_eq!(restore.operation, "restore_original");
    f.app
        .apply_capture_edit_local(&f.account, &restore.id, "apply")
        .unwrap();
    let after = f
        .app
        .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert!(
        f.app
            .capture_media_page_local(&f.account, &capture.id, 0)
            .unwrap()[0]
            .matches_message
    );
    assert!(f.repo.files(&f.account).unwrap().is_empty());
}
#[test]
fn specialized_records_use_full_archive_and_preserve_display_only_fields() {
    let f = Fixture::new();
    let capture = complete(&f);
    let page = f
        .app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let mut patch = f
        .app
        .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
        .unwrap();
    patch.message_type = "49".into();
    patch.text = None;
    patch.extra.as_mut().unwrap().metadata=Some("<msg><appmsg><type>2000</type><title>=自动化测试</title><wcpayinfo><feedesc>￥1.00</feedesc><transferid>private</transferid></wcpayinfo></appmsg></msg>".into());
    let c = f
        .app
        .preview_capture_edit_local(&f.account, &capture.id, 0, 0, page.revision, patch)
        .unwrap();
    f.app
        .apply_capture_edit_local(&f.account, &c.id, "apply")
        .unwrap();
    let query = RecordQuery {
        kind: Some(werc_domain::message_card::records::RecordKind::Transfer),
        text: "自动化测试".into(),
        ..Default::default()
    };
    let result = f
        .app
        .capture_records_local(&f.account, &capture.id, &query, None, "owned-records-list")
        .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.data[0].detail.fields["消息显示金额"], "￥1.00");
    assert!(!serde_json::to_string(&result).unwrap().contains("private"));
    let mut csv = Vec::new();
    assert_eq!(
        f.app
            .export_capture_records_local(
                &f.account,
                &capture.id,
                &query,
                "csv",
                "owned-records-export",
                &mut csv
            )
            .unwrap(),
        1
    );
    let csv = String::from_utf8(csv).unwrap();
    assert!(csv.contains("'=自动化测试"));
    assert!(csv.contains("edited_local"));
    for format in ["json", "jsonl", "xlsx"] {
        let mut out = Vec::new();
        assert_eq!(
            f.app
                .export_capture_records_local(
                    &f.account,
                    &capture.id,
                    &query,
                    format,
                    &format!("owned-records-{format}"),
                    &mut out
                )
                .unwrap(),
            1
        );
        if format == "json" {
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(v["records"][0]["local_status"], "edited_local");
            assert_eq!(v["count"], 1);
        }
        if format == "xlsx" {
            assert!(out.starts_with(b"PK"));
            if let Some(path) = std::env::var_os("WERC_RECORDS_XLSX_FIXTURE") {
                std::fs::write(path, &out).unwrap();
            }
        }
    }
    let only = RecordQuery {
        selected: vec![result.data[0].position.clone()],
        ..Default::default()
    };
    assert_eq!(
        f.app
            .capture_records_local(
                &f.account,
                &capture.id,
                &only,
                None,
                "owned-records-selected"
            )
            .unwrap()
            .data
            .len(),
        1
    );
    let none = RecordQuery {
        raw_status: Some("999".into()),
        ..Default::default()
    };
    assert!(
        f.app
            .capture_records_local(&f.account, &capture.id, &none, None, "owned-records-status")
            .unwrap()
            .data
            .is_empty()
    );
    let page = f
        .app
        .source_capture_page_local(&f.account, &capture.id, 0)
        .unwrap();
    let mut patch = f
        .app
        .capture_edit_fields_local(&f.account, &capture.id, 0, 0)
        .unwrap();
    patch.text = Some("自动化测试：导出过程中修改本地副本".into());
    let change = f
        .app
        .preview_capture_edit_local(&f.account, &capture.id, 0, 0, page.revision, patch)
        .unwrap();
    struct EditDuringWrite<'a> {
        app: &'a Application,
        account: &'a str,
        change: &'a str,
        changed: bool,
    }
    impl std::io::Write for EditDuringWrite<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.changed
                && bytes
                    .windows(b"edited_local".len())
                    .any(|w| w == b"edited_local")
            {
                self.app
                    .apply_capture_edit_local(self.account, self.change, "apply")
                    .unwrap();
                self.changed = true;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = EditDuringWrite {
        app: &f.app,
        account: &f.account,
        change: &change.id,
        changed: false,
    };
    assert_eq!(
        f.app
            .export_capture_records_local(
                &f.account,
                &capture.id,
                &query,
                "csv",
                "owned-records-mutated-export",
                &mut writer
            )
            .unwrap_err()
            .code,
        "DOCUMENT_CONFLICT"
    );
    assert!(writer.changed);
}
#[tokio::test]
async fn archived_binary_media_requires_media_scope_and_roundtrips_bytes() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    f.app.enable_api(true);
    *f.reader.body.lock().unwrap() = format!(
        "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabb\" length=\"100\"/></msg>",
        "11".repeat(16)
    );
    let capture = complete(&f);
    let media = f
        .app
        .archive_capture_media_local(&f.account, &capture.id, 0, 0, "owned-http-media-save")
        .unwrap();
    for allowed in [false, true] {
        let mut scopes = vec!["messages:read".into()];
        if allowed {
            scopes.push("media:read".into());
        }
        let token = f
            .app
            .issue_token_local("owned archive", vec![f.account.clone()], scopes, 1)
            .unwrap();
        let request=Request::builder().method("POST").uri("/api/v1/messages/recovery/media/content")
            .header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json")
            .body(Body::from(serde_json::json!({"account_id":f.account,"id":capture.id,"index":0,"row":0,"request_id":"owned-media-download"}).to_string())).unwrap();
        let response = werc_http_api::router(f.app.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), if allowed { 200 } else { 403 });
        if allowed {
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["content-type"], "image/png");
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert_eq!(hex::encode(sha2::Sha256::digest(&bytes)), media.sha256);
        }
    }
}


#[test]
fn contact_timeline_orders_exact_mentions_pending_contacts_and_rejects_changed_pages() {
    use aes_gcm::{Aes256Gcm,KeyInit,Nonce,aead::{Aead,Payload}};
    use werc_application::capture_timeline::*;
    let f=Fixture::new();
    let capture=complete(&f);
    let login=f.reader.context.lock().unwrap().sender_id.clone();
    // Replace an owned encrypted fixture page with independently specified source observations.
    let mut document=f.repo.library_get(&f.account,&format!("{}:0",capture.id)).unwrap();
    let key=f.reader.unprotect_key(&format!("WeRC/library/v1/{}/{}",f.account,document.header.id),&document.wrapped_key).unwrap();
    let cipher=Aes256Gcm::new_from_slice(&key).unwrap();
    let aad=serde_json::to_vec(&document.header).unwrap();
    let bytes=cipher.decrypt(Nonce::from_slice(&document.nonce),Payload {msg:&document.ciphertext,aad:&aad}).unwrap();
    let mut batch:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
    let samples=vec![
        ("peer-in",Some("peer-in"),"请回应",None),
        ("peer-out",Some(login.as_str()),"仍等回复",None),
        ("peer-unknown",Some("peer-unknown"),"旧的问候",None),
        ("peer-unknown",None,"身份缺失",None),
        ("peer-replied",Some("peer-replied"),"找你",None),
        ("peer-replied",Some(login.as_str()),"已回复",None),
        ("owned@im.chatroom",Some("member"),"提及本人",Some(vec![login.clone()])),
        ("owned@im.chatroom",Some("member"),"提及所有人",Some(vec!["notify@all".into()])),
        ("owned@im.chatroom",Some("member"),"@昵称不是提及证据",Some(vec![format!("{login}_other")])),
        ("owned@im.chatroom",Some("member"),"旧来源无字段",None),
        ("gh_owned",Some("gh_owned"),"公众号不计待回复",None),
        ("peer-stranger",Some("another"),"不符会话的发送者",None),
    ];
    for (index,(username,sender,text,mentions)) in samples.iter().enumerate() {
        batch["rows"][index]=serde_json::json!({"username":username,"source_index":0,"message":{"local_id":(index+1).to_string(),"server_id":(9900+index).to_string(),"timestamp":1701000000+index as i64,"message_type":"1","text":text,"content_status":"text","sender_username":sender,"at_user_list":mentions}});
    }
    batch["rows"][12]=serde_json::json!({"username":"peer-in","source_index":0,"message":{"local_id":"13","server_id":"9913","timestamp":1701000050,"message_type":"10000","text":"系统通知","content_status":"text","sender_username":login}});
    for (index,sender) in [(13,login.as_str()),(14,"peer-tied")] {
        batch["rows"][index]=serde_json::json!({"username":"peer-tied","source_index":0,"message":{"local_id":(index+1).to_string(),"server_id":(9910+index).to_string(),"timestamp":1701000060,"message_type":"1","text":"同秒双方发言","content_status":"text","sender_username":sender}});
    }
    document.header.revision+=1;
    document.nonce=vec![0x79;12];
    document.ciphertext=cipher.encrypt(Nonce::from_slice(&document.nonce),Payload {msg:&serde_json::to_vec(&batch).unwrap(),aad:&serde_json::to_vec(&document.header).unwrap()}).unwrap();
    f.repo.library_save(&[document],None).unwrap();
    let base=TimelineQuery {from:Some(1701000000),until:Some(1701000100),..Default::default()};
    let read=|query:&TimelineQuery|f.app.capture_timeline_local(&f.account,&capture.id,query,None,"owned-contact-timeline").unwrap();
    let all=read(&base);
    assert_eq!(all.data.len(),15);
    assert_eq!((all.summary.waiting_me,all.summary.waiting_them),(1,2));
    assert_eq!((all.summary.mentions,all.summary.mention_groups,all.summary.group_rows_without_metadata),(2,1,1));
    assert!(all.data.windows(2).all(|w|w[0].timestamp>=w[1].timestamp));
    let mentions=read(&TimelineQuery {mode:TimelineMode::Mentions,..base.clone()});
    assert_eq!(mentions.data.len(),2);
    assert!(mentions.data[0].mentions_all && mentions.data[1].mentions_me);
    let pending=read(&TimelineQuery {mode:TimelineMode::WaitingMe,..base.clone()});
    assert_eq!(pending.data.len(),1);assert_eq!(pending.data[0].username,"peer-in");
    assert_eq!(pending.data[0].row,0);
    assert!(read(&TimelineQuery {mode:TimelineMode::WaitingMe,text:"旧的问候".into(),..base.clone()}).data.is_empty());
    let mut page=read(&TimelineQuery::default());
    assert_eq!(page.data.len(),100);
    let cursor=page.next.take().unwrap();
    let next=f.app.capture_timeline_local(&f.account,&capture.id,&TimelineQuery::default(),Some(&cursor),"owned-contact-next").unwrap();
    assert!(!next.data.iter().any(|b| page.data.iter().any(|a|a.page==b.page && a.row==b.row)));
    assert_eq!(f.app.capture_timeline_local(&f.account,&capture.id,&base,Some(&cursor),"owned-changed-query").err().unwrap().code,"CURSOR_STALE");
    let source=f.app.source_capture_page_local(&f.account,&capture.id,0).unwrap();
    let mut patch=f.app.capture_edit_fields_local(&f.account,&capture.id,0,6).unwrap();
    patch.extra.as_mut().unwrap().username="another@chatroom".into();
    let preview=f.app.preview_capture_edit_local(&f.account,&capture.id,0,6,source.revision,patch).unwrap();
    f.app.apply_capture_edit_local(&f.account,&preview.id,"apply").unwrap();
    assert!(f.app.source_capture_page_local(&f.account,&capture.id,0).unwrap().rows[6].message.at_user_list.is_none());
    assert_eq!(f.app.capture_timeline_local(&f.account,&capture.id,&TimelineQuery::default(),Some(&cursor),"owned-changed-source").err().unwrap().code,"CURSOR_STALE");
    f.app.cancel_browse_local("owned-contact-cancel").unwrap();
    assert_eq!(f.app.capture_timeline_local(&f.account,&capture.id,&base,None,"owned-contact-cancel").err().unwrap().code,"QUERY_CANCELED");

}


#[tokio::test]
async fn social_association_uses_exact_owner_dates_and_separate_read_permissions() {
    use aes_gcm::{Aes256Gcm,KeyInit,Nonce,aead::{Aead,Payload}};
    use axum::{body::{Body,to_bytes},http::Request};
    use tower::ServiceExt;
    let f=Fixture::new();
    let capture=complete(&f);
    f.app.configure_moment_preservation_local(&f.account,true).unwrap();
    for _ in 0..3 { if !f.app.step_moment_preservation_local(&f.account).unwrap().scanning {break;} }
    let login=f.reader.context.lock().unwrap().sender_id.clone();
    let headers=f.repo.library_list(&f.account,"moment_preserved",None,100).unwrap().data;
    assert_eq!(headers.len(),31);
    let interaction=|username:&str,time:Option<i64>,deleted:bool|serde_json::json!({"username":username,"nickname":"自有样本","text":"评论","timestamp":time,"id":null,"id64":null,"reply_id":null,"reply_id64":null,"reply_username":"","deleted":deleted,"content_type":null,"image_count":null,"emoji_count":null});
    for (index,header) in headers.iter().take(3).enumerate() {
        let mut doc=f.repo.library_get(&f.account,&header.id).unwrap();
        let key=f.reader.unprotect_key(&format!("WeRC/library/v1/{}/{}",f.account,header.id),&doc.wrapped_key).unwrap();
        let cipher=Aes256Gcm::new_from_slice(&key).unwrap();
        let plain=cipher.decrypt(Nonce::from_slice(&doc.nonce),Payload {msg:&doc.ciphertext,aad:&serde_json::to_vec(&doc.header).unwrap()}).unwrap();
        let mut item:serde_json::Value=serde_json::from_slice(&plain).unwrap();
        item["moment"]["anchor"]["username"]=serde_json::json!(if index==0 {login.as_str()} else {"owned-social-peer"});
        item["moment"]["content"]["details"]["likes"]=if index==0 {serde_json::json!([interaction("owned-social-peer",Some(1700000001),false),interaction("owned-social-peer",Some(1700000001),false),interaction("owned-deleted",Some(1700000001),true)])} else {serde_json::json!([interaction(&login,Some(1700000002),false)])};
        item["moment"]["content"]["details"]["comments"]=serde_json::json!([interaction(if index==0 {"owned-social-peer"} else {&login},None,false)]);
        if index==2 {item["login"]=serde_json::json!("different-owner");}
        doc.header.revision+=1;doc.nonce=vec![0x69;12];
        doc.ciphertext=cipher.encrypt(Nonce::from_slice(&doc.nonce),Payload {msg:&serde_json::to_vec(&item).unwrap(),aad:&serde_json::to_vec(&doc.header).unwrap()}).unwrap();
        f.repo.library_save(&[doc],None).unwrap();
    }
    let filter=CaptureFilter {conversations:vec!["owned-social-peer".into()],from:Some(1700000000),until:Some(1700000010),..Default::default()};
    let report=f.app.capture_social_report_local(&f.account,&capture.id,&filter,"owned-social-report").unwrap();
    assert_eq!((report.own_posts,report.observed_posts,report.unmatched_login_posts),(1,2,1));
    assert_eq!(report.contacts.len(),1);
    let peer=&report.contacts[0];
    assert_eq!((peer.likes_given,peer.likes_received,peer.comments_given,peer.comments_received),(1,1,0,0));
    assert_eq!(report.interactions_without_time,2);
    let unbounded=CaptureFilter {from:None,until:None,..filter.clone()};
    let report=f.app.capture_social_report_local(&f.account,&capture.id,&unbounded,"owned-social-no-dates").unwrap();
    assert_eq!((report.contacts[0].comments_given,report.contacts[0].comments_received),(1,1));
    for format in ["txt","html","json"] {let out=f.app.export_capture_social_local(&f.account,&capture.id,&filter,format,"owned-social-export").unwrap();assert!(String::from_utf8(out).unwrap().contains("owned-social-peer"));}
    f.app.enable_api(true);
    for (scopes,expected) in [(vec!["messages:read".into()],403),(vec!["messages:read".into(),"moments:read".into()],200)] {
        let token=f.app.issue_token_local("owned social read",vec![f.account.clone()],scopes,1).unwrap();
        let request=Request::builder().method("POST").uri("/api/v1/messages/recovery").header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json").body(Body::from(serde_json::json!({"action":"social_report","account_id":f.account,"id":capture.id,"filter":filter,"request_id":"owned-social-api-0001"}).to_string())).unwrap();
        let response=werc_http_api::router(f.app.clone()).oneshot(request).await.unwrap();assert_eq!(response.status().as_u16(),expected);
        if expected==200 {let result:serde_json::Value=serde_json::from_slice(&to_bytes(response.into_body(),1024*1024).await.unwrap()).unwrap();assert_eq!(result["contacts"][0]["likes_received"],1);}
    }
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[tokio::test]
async fn message_locator_requires_exact_scope_and_identity_and_rejects_conflicting_copies() {
    use aes_gcm::{Aes256Gcm,KeyInit,Nonce,aead::{Aead,Payload}};
    use axum::{body::{Body,to_bytes},http::Request};
    use tower::ServiceExt;
    let f=Fixture::new();let capture=complete(&f);
    let mut doc=f.repo.library_get(&f.account,&format!("{}:0",capture.id)).unwrap();
    let key=f.reader.unprotect_key(&format!("WeRC/library/v1/{}/{}",f.account,doc.header.id),&doc.wrapped_key).unwrap();
    let cipher=Aes256Gcm::new_from_slice(&key).unwrap();
    let plain=cipher.decrypt(Nonce::from_slice(&doc.nonce),Payload {msg:&doc.ciphertext,aad:&serde_json::to_vec(&doc.header).unwrap()}).unwrap();
    let mut batch:serde_json::Value=serde_json::from_slice(&plain).unwrap();
    let original=serde_json::json!({"username":"owned-locate@chatroom","source_index":0,"message":{"local_id":"1","server_id":"-1","timestamp":1702000000,"message_type":"1","text":"自动化测试原消息","content_status":"text","sender_username":"owned-member"}});
    batch["rows"][0]=original.clone();
    batch["rows"][1]=original.clone();batch["rows"][1]["username"]=serde_json::json!("another@chatroom");
    let persist=|doc:&mut werc_domain::library::ProtectedDocument,batch:&serde_json::Value| {
        doc.header.revision+=1;doc.nonce=vec![0x30+doc.header.revision as u8;12];
        doc.ciphertext=cipher.encrypt(Nonce::from_slice(&doc.nonce),Payload{msg:&serde_json::to_vec(batch).unwrap(),aad:&serde_json::to_vec(&doc.header).unwrap()}).unwrap();
        f.repo.library_save(std::slice::from_ref(doc),None).unwrap();
    };
    persist(&mut doc,&batch);
    let locate=||f.app.capture_message_location_local(&f.account,&capture.id,"owned-locate@chatroom","18446744073709551615","owned-locate-read");
    let found=locate().unwrap();assert_eq!((found.position.page,found.position.row,found.copies),(0,0,1));
    assert_eq!(f.app.capture_message_location_local(&f.account,&capture.id,"owned-locate","18446744073709551615","owned-locate-miss").err().unwrap().code,"MESSAGE_NOT_IN_CAPTURE");
    assert_eq!(f.app.capture_message_location_local(&f.account,&capture.id,"owned-locate@chatroom","0","owned-locate-zero").err().unwrap().code,"INVALID_ARGUMENT");
    batch["rows"][2]=original.clone();batch["rows"][2]["source_index"]=serde_json::json!(1);batch["rows"][2]["message"]["local_id"]=serde_json::json!("2");
    persist(&mut doc,&batch);assert_eq!(locate().unwrap().copies,2);
    f.app.enable_api(true);
    let token=f.app.issue_token_local("owned locate read",vec![f.account.clone()],vec!["messages:read".into()],1).unwrap();
    let request=Request::builder().method("POST").uri("/api/v1/messages/recovery").header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json").body(Body::from(serde_json::json!({"action":"locate_message","account_id":f.account,"id":capture.id,"username":"owned-locate@chatroom","server_id":"18446744073709551615","request_id":"owned-locate-api-0001"}).to_string())).unwrap();
    let response=werc_http_api::router(f.app.clone()).oneshot(request).await.unwrap();assert_eq!(response.status().as_u16(),200);
    let value:serde_json::Value=serde_json::from_slice(&to_bytes(response.into_body(),1024*1024).await.unwrap()).unwrap();assert_eq!(value["copies"],2);
    batch["rows"][2]["message"]["text"]=serde_json::json!("冲突的正文");persist(&mut doc,&batch);
    assert_eq!(locate().err().unwrap().code,"MESSAGE_ID_AMBIGUOUS");
    f.app.cancel_browse_local("owned-locate-cancel").unwrap();assert_eq!(f.app.capture_message_location_local(&f.account,&capture.id,"owned-locate@chatroom","-1","owned-locate-cancel").err().unwrap().code,"QUERY_CANCELED");
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

// Simulate a concurrent authenticated document update at a precise read boundary.
fn advance_owned_document(repo: &SqliteRepository, account: &str, id: &str) {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::{Aead, Payload}};
    let mut doc = repo.library_get(account, id).unwrap();
    let key = WindowsWechatProvider.unprotect_key(
        &format!("WeRC/library/v1/{account}/{id}"), &doc.wrapped_key).unwrap();
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
    let plain = zeroize::Zeroizing::new(cipher.decrypt(Nonce::from_slice(&doc.nonce),
        Payload { msg: &doc.ciphertext, aad: &serde_json::to_vec(&doc.header).unwrap() }).unwrap());
    doc.header.revision += 1;
    doc.nonce = vec![0x91; 12];
    doc.ciphertext = cipher.encrypt(Nonce::from_slice(&doc.nonce),
        Payload { msg: &plain, aad: &serde_json::to_vec(&doc.header).unwrap() }).unwrap();
    repo.library_save(&[doc], None).unwrap();
}

#[test]
fn capture_consistency_rejects_root_update_during_export_and_chat_update_during_social() {
    let f = Fixture::new();
    let capture = complete(&f);
    let repo = f.repo.clone(); let account = f.account.clone(); let root = capture.id.clone();
    *f.reader.on_unprotect.lock().unwrap() = Some((
        format!("WeRC/library/v1/{}/{}:0", f.account, capture.id),
        Box::new(move || advance_owned_document(&repo, &account, &root))));
    assert_eq!(f.app.capture_report_local(&f.account, &capture.id, &CaptureFilter::default(),
        "owned-consistency-report").err().unwrap().code, "DOCUMENT_CONFLICT");
    // The updated document remains valid on the next request.
    f.app.capture_report_local(&f.account, &capture.id, &CaptureFilter::default(),
        "owned-consistency-retry").unwrap();
    f.app.configure_moment_preservation_local(&f.account, true).unwrap();
    f.app.step_moment_preservation_local(&f.account).unwrap();
    let preserved = f.repo.library_list(&f.account, "moment_preserved", None, 100).unwrap();
    assert!(!preserved.data.is_empty());
    for target in [capture.id.clone(), format!("{}:0", capture.id)] {
        let repo = f.repo.clone(); let account = f.account.clone();
        *f.reader.on_unprotect.lock().unwrap() = Some((
            format!("WeRC/library/v1/{}/{}", f.account, preserved.data[0].id),
            Box::new(move || advance_owned_document(&repo, &account, &target))));
        assert_eq!(f.app.capture_social_report_local(&f.account, &capture.id,
            &CaptureFilter::default(), "owned-consistency-social").err().unwrap().code,
            "DOCUMENT_CONFLICT");
    }
    f.app.capture_social_report_local(&f.account, &capture.id, &CaptureFilter::default(),
        "owned-consistency-social-retry").unwrap();
}

#[test]
fn account_removal_requires_stopped_plans_and_pending_tasks_to_finish() {
    let f=Fixture::new();let capture=complete(&f);
    let plan=f.app.save_export_plan_local(&f.account,None,0,plan_spec(&capture.id)).unwrap();
    assert_eq!(f.app.remove_account_local(&f.account).err().unwrap().code,"ACCOUNT_PLAN_ACTIVE");
    assert!(f.repo.library_get(&f.account,&capture.id).is_ok());
    let mut spec=plan.plan.spec.clone();spec.enabled=false;
    f.app.save_export_plan_local(&f.account,Some(&plan.id),plan.revision,spec).unwrap();
    let asset=f.app.upload_audio_local(&f.account,owned_voice_wav()).unwrap();
    let operation=f.app.inspect_audio_task_local(&f.account,&asset.id,"owned-removal-task-once").unwrap();
    assert_eq!(f.app.remove_account_local(&f.account).err().unwrap().code,"ACCOUNT_BUSY");
    assert!(f.repo.library_get(&f.account,&capture.id).is_ok());
    f.repo.cancel_operation(&f.account,&operation.id).unwrap();
    f.app.remove_account_local(&f.account).unwrap();
    assert!(f.repo.accounts().unwrap().is_empty());
    assert!(f.repo.library_get(&f.account,&capture.id).is_err());
}
