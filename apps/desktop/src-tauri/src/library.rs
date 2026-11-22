use super::*;
use werc_application::capture_tools::*;
use werc_application::{library::*, source_capture::*};
#[tauri::command]
pub(super) async fn capture_message_location(state:State<'_,DesktopState>,account_id:String,id:String,username:String,server_id:String,request_id:String)->Result<werc_application::source_capture::MessageLocation,AppError>{
    let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||app.capture_message_location_local(&account_id,&id,&username,&server_id,&request_id)).await.map_err(|_|AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_names_status(state:State<'_,DesktopState>,account_id:String,id:String)->Result<CaptureNamesSummary,AppError>{
    let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||app.capture_names_status_local(&account_id,&id)).await.map_err(|_|AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_names_refresh(state:State<'_,DesktopState>,account_id:String,id:String,view_id:String,request_id:String)->Result<CaptureNamesSummary,AppError>{
    let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||app.refresh_capture_names_local(&account_id,&id,&view_id,&request_id)).await.map_err(|_|AppError::internal())?
}

#[tauri::command]
pub(super) async fn save_visual_png(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    path: String,
    bytes: Vec<u8>,
) -> Result<String, AppError> {
    let downloads = handle.path().download_dir().map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let app = state.application.clone();
    let bytes = zeroize::Zeroizing::new(bytes);
    tauri::async_runtime::spawn_blocking(move || {
        if !app.accounts_local()?.iter().any(|a| a.id == account_id) {
            return Err(AppError::new("ACCOUNT_NOT_FOUND", "所选账号不存在"));
        }
        let (_, png) = werc_domain::image::canonical_png(&bytes)?;
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != Some("png") {
            return Err(AppError::new("INVALID_ARGUMENT", "图片文件扩展名应为.png"));
        }
        export_file::stream_new(&destination, |out| {
            std::io::Write::write_all(out, &png).map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "图片保存失败"))?;
            Ok(())
        })?;
        Ok(destination.to_string_lossy().into_owned())
    }).await.map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn export_transcript(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    format: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entry = app
            .library_item_local(&account_id, &id, "transcript")?
            .entry;
        let bytes = match format.as_str() {
            "txt" => entry.text.into_bytes(),
            "json" => serde_json::to_vec_pretty(&entry.transcript.ok_or_else(|| {
                AppError::new("TRANSCRIPT_TIMINGS_MISSING", "旧转写稿没有字幕时间")
            })?)
            .map_err(|_| AppError::internal())?,
            "srt" => entry
                .transcript
                .ok_or_else(|| AppError::new("TRANSCRIPT_TIMINGS_MISSING", "旧转写稿没有字幕时间"))?
                .srt()
                .into_bytes(),
            _ => {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "转写稿支持TXT、SRT、JSON",
                ));
            }
        };
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|v| v.to_str()) != Some(&format) {
            return Err(AppError::new("INVALID_ARGUMENT", "转写稿文件名与格式不符"));
        }
        export_file::stream_new(&destination, |file| {
            use std::io::Write;
            file.write_all(&bytes)
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "保存转写稿失败"))?;
            Ok(bytes.len())
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_report_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    filter: CaptureFilter,
    request_id: String,
    format: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|v| v.to_str()) != Some(&format) {
            return Err(AppError::new("INVALID_ARGUMENT", "报告文件名与格式不符"));
        }
        let report = app.capture_report_local(&account_id, &id, &filter, &request_id)?;
        let bytes = report.export(&format)?;
        export_file::stream_new(&destination, |file| {
            use std::io::Write;
            file.write_all(&bytes)
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "报告保存失败"))?;
            Ok(bytes.len())
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_edit_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    revision: i64,
    patch: CapturePatch,
) -> Result<ChangePreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preview_capture_edit_local(&account_id, &id, index, row, revision, patch)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_edit_apply(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    action: String,
) -> Result<ChangePreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.apply_capture_edit_local(&account_id, &id, &action)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_edits(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
) -> Result<(Vec<ChangePreview>, Option<i64>), AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.capture_edits_local(&account_id, before))
        .await
        .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn asr_status(
    state: State<'_, DesktopState>,
) -> Result<werc_domain::asr::AsrStatus, AppError> {
    state.application.asr_status_local()
}
#[tauri::command]
pub(super) async fn transcribe_audio(
    state: State<'_, DesktopState>,
    account_id: String,
    source: werc_domain::asr::AudioSource,
    model: String,
    language: String,
    request_id: String,
) -> Result<werc_domain::asr::Transcript, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.transcribe_local(&account_id, &source, &model, &language, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn library_backup(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    path: String,
    password: String,
    request_id: String,
    restore: bool,
) -> Result<u64, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let app = state.application.clone();
    let password = zeroize::Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != Some("werc") {
            return Err(AppError::new("INVALID_ARGUMENT", "备份文件扩展名应为.werc"));
        }
        if restore {
            let file = std::fs::File::open(&destination)
                .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "备份文件无法打开"))?;
            app.restore_library_local(
                &account_id,
                &password,
                &request_id,
                &mut std::io::BufReader::new(file),
            )
        } else {
            export_file::stream_new(&destination, |file| {
                app.backup_library_local(&account_id, &password, &request_id, file)
            })
        }
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_report(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    filter: CaptureFilter,
    request_id: String,
) -> Result<CaptureReport, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_report_local(&account_id, &id, &filter, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(Serialize)]
pub(super) struct SavedCaptureExport {
    #[serde(flatten)]
    summary: CaptureExportSummary,
    saved_path: String,
    skipped_identical: bool,
}
#[tauri::command]
pub(super) async fn capture_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    format: String,
    options: CaptureExportOptions,
    path: String,
    request_id: String,
    conflict_policy: Option<export_file::ConflictPolicy>,
) -> Result<SavedCaptureExport, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != capture_export_extension(&format) {
            return Err(AppError::new("INVALID_ARGUMENT", "文件扩展名与格式不符"));
        }
        let (summary, saved, skipped_identical) = export_file::stream_with_policy(&destination, conflict_policy.unwrap_or_default(), |file| {
            app.export_capture_local(&account_id, &id, &format, &options, &request_id, file)
        })?;
        Ok(SavedCaptureExport { summary, saved_path: saved.to_string_lossy().into_owned(), skipped_identical })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn library_page(
    state: State<'_, DesktopState>,
    account_id: String,
    category: String,
    before: Option<i64>,
) -> Result<LibraryPage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.library_page_local(&account_id, &category, before)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn library_save(
    state: State<'_, DesktopState>,
    account_id: String,
    id: Option<String>,
    category: String,
    revision: i64,
    entry: LibraryEntry,
) -> Result<LibraryItem, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.library_save_local(&account_id, id.as_deref(), &category, revision, entry)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn library_delete(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    category: String,
    revision: i64,
) -> Result<(), AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.library_delete_local(&account_id, &id, &category, revision)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_start(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: Option<String>,
    from: Option<i64>,
    until: Option<i64>,
) -> Result<CaptureStatus, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match view_id {
            Some(view_id) => app.start_source_capture_local(&account_id, &view_id, from, until),
            None => app.start_enrolled_capture_local(&account_id, from, until),
        }
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_step(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    request_id: String,
) -> Result<CaptureStatus, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.step_source_capture_local(&account_id, &id, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_list(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
) -> Result<CaptureList, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.source_captures_local(&account_id, before))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_page(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
) -> Result<CapturePage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.source_capture_page_local(&account_id, &id, index)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_media_page(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
) -> Result<Vec<MediaRow>, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_media_page_local(&account_id, &id, index)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_media_progress(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<MediaProgress, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.capture_media_progress_local(&account_id, &id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_media_step(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    restart: bool,
    request_id: String,
) -> Result<MediaProgress, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.step_capture_media_local(&account_id, &id, restart, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_media_save(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    request_id: String,
) -> Result<ArchivedMedia, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.archive_capture_media_local(&account_id, &id, index, row, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_media_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    request_id: String,
    path: String,
) -> Result<ArchivedMedia, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        export_file::stream_new(&destination, |file| {
            app.export_capture_media_local(&account_id, &id, index, row, &request_id, file)
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_bundle_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    request_id: String,
    path: String,
) -> Result<BundleSummary, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != Some("zip") {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "归档文件须使用.zip扩展名",
            ));
        }
        export_file::stream_new(&destination, |file| {
            app.export_capture_bundle_local(&account_id, &id, &request_id, file)
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_transcript_progress(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    model: String,
    language: String,
) -> Result<MediaProgress, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_transcript_progress_local(&account_id, &id, &model, &language)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_transcript_step(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    model: String,
    language: String,
    restart: bool,
    request_id: String,
) -> Result<MediaProgress, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.step_capture_transcripts_local(
            &account_id,
            &id,
            &model,
            &language,
            restart,
            &request_id,
        )
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_status(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<CaptureStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.source_capture_status_local(&account_id, &id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_edit_fields(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
) -> Result<CapturePatch, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_edit_fields_local(&account_id, &id, index, row)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_delete_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    revision: i64,
) -> Result<ChangePreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preview_capture_delete_local(&account_id, &id, index, row, revision)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_add_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    username: String,
    text: String,
    timestamp: i64,
) -> Result<ChangePreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preview_capture_add_local(&account_id, &id, &username, &text, timestamp)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_media_audit(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    request_id: String,
) -> Result<MediaAudit, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.audit_capture_media_local(&account_id, &id, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_media_catalog(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    query: MediaQuery,
    after: Option<MediaPosition>,
    request_id: String,
) -> Result<MediaCatalog, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_media_catalog_local(&account_id, &id, &query, after, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(serde::Serialize)]
pub(super) struct MediaPreview {
    mime: String,
    bytes: Vec<u8>,
}
#[tauri::command]
pub(super) async fn capture_media_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    request_id: String,
) -> Result<MediaPreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let content = app.capture_media_content_local(&account_id, &id, index, row, &request_id)?;
        if content.mime == "application/octet-stream" {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "附件请保存后用对应程序打开",
            ));
        }
        Ok(MediaPreview {
            mime: content.mime.into(),
            bytes: content.file.bytes.to_vec(),
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_original_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    revision: i64,
) -> Result<ChangePreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preview_capture_original_local(&account_id, &id, index, row, revision)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_records(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    query: RecordQuery,
    after: Option<MediaPosition>,
    request_id: String,
) -> Result<RecordCatalog, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.capture_records_local(&account_id, &id, &query, after, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_records_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    query: RecordQuery,
    format: String,
    request_id: String,
    path: String,
) -> Result<u64, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        export_file::stream_new(&destination, |out| {
            app.export_capture_records_local(&account_id, &id, &query, &format, &request_id, out)
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_timeline(state:State<'_,DesktopState>,account_id:String,id:String,query:werc_application::capture_timeline::TimelineQuery,after:Option<werc_application::capture_timeline::TimelineCursor>,request_id:String)->Result<werc_application::capture_timeline::ContactTimeline,AppError>{
 let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||app.capture_timeline_local(&account_id,&id,&query,after.as_ref(),&request_id)).await.map_err(|_|AppError::internal())?
}

#[tauri::command]
pub(super) async fn capture_social_report(state:State<'_,DesktopState>,account_id:String,id:String,filter:CaptureFilter,request_id:String)->Result<werc_application::capture_social::CaptureSocialReport,AppError>{
 let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||app.capture_social_report_local(&account_id,&id,&filter,&request_id)).await.map_err(|_|AppError::internal())?
}
#[tauri::command]
pub(super) async fn capture_social_report_export(handle:tauri::AppHandle,state:State<'_,DesktopState>,account_id:String,id:String,filter:CaptureFilter,request_id:String,format:String,path:String)->Result<(),AppError>{
 let downloads=handle.path().download_dir().map_err(|_|AppError::internal())?;
 let app=state.application.clone();tauri::async_runtime::spawn_blocking(move||{
  let bytes=zeroize::Zeroizing::new(app.export_capture_social_local(&account_id,&id,&filter,&format,&request_id)?);
  let destination=export_file::destination(&path,&downloads)?;
  export_file::stream_new(&destination,|out|{use std::io::Write;out.write_all(&bytes).map_err(|_|AppError::new("EXPORT_WRITE_FAILED","关联报告保存失败"))})
 }).await.map_err(|_|AppError::internal())?
}
