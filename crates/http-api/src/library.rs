use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureExportInput {
    account_id: String,
    id: String,
    format: String,
    #[serde(default)]
    options: werc_application::capture_tools::CaptureExportOptions,
    request_id: String,
}
pub(super) async fn capture_export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<CaptureExportInput>,
) -> Result<Response, ApiError> {
    use werc_application::capture_tools::capture_export_extension;
    fn access(p: &Principal, a: &str, media: bool, names: bool, transcripts: bool) -> Result<(), AppError> {
        p.require("messages:read")?;
        p.require("exports:create")?;
        if media {
            p.require("media:read")?;
        }
        if names { p.require("contacts:read")?; }
        if transcripts { p.require("sources:read")?; }
        p.require_account(a)
    }
    let media = input.format == "chatlab_zip";
    let names = !input.options.redact_identifiers && (
        (matches!(input.format.as_str(), "txt"|"md"|"html"|"xlsx"|"chatlab_json"|"chatlab_jsonl"|"chatlab_zip")
            && !matches!(input.options.display_name.as_str(), ""|"username"))
        || (input.format == "xlsx" && (input.options.xlsx_columns.is_empty()
            || input.options.xlsx_columns.iter().any(|c| matches!(c.as_str(), "nickname"|"remark"|"group_nickname"))))
        || (media && input.options.include_avatars));
    let transcripts = input.options.include_transcripts && matches!(input.format.as_str(), "xlsx"|"chatlab_zip")
        && !input.options.redact_identifiers && !input.options.omit_text && input.options.replacements.is_empty();
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id, media, names, transcripts)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let extension = capture_export_extension(&input.format)
        .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "导出格式无效"))?;
    let mime = match extension {
        "zip" => "application/zip",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "json" => "application/json",
        "jsonl" => "application/x-ndjson",
        "csv" => "text/csv; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    };
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let bytes = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, media, names, transcripts)?;
        struct Bounded(zeroize::Zeroizing<Vec<u8>>);
        impl std::io::Write for Bounded {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if self.0.len().saturating_add(data.len()) > 64 * 1024 * 1024 {
                    return Err(std::io::Error::other("export limit"));
                }
                self.0.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = Bounded(zeroize::Zeroizing::new(Vec::new()));
        app.export_capture_local(
            &input.account_id,
            &input.id,
            &input.format,
            &input.options,
            &request,
            &mut out,
        )?;
        Ok(out.0)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, media, names, transcripts)?;
    Ok((
        [
            ("content-type", mime),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            (
                "content-disposition",
                &format!("attachment; filename=\"WeRC-capture.{extension}\""),
            ),
        ],
        bytes.to_vec(),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TranscribeInput {
    account_id: String,
    source: werc_domain::asr::AudioSource,
    model: String,
    language: String,
    request_id: String,
}
pub(super) async fn asr_status(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
) -> Result<Json<werc_domain::asr::AsrStatus>, ApiError> {
    p.require("media:read")?;
    Ok(Json(state.application.asr_status_local()?))
}
pub(super) async fn transcribe(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<TranscribeInput>,
) -> Result<Json<werc_domain::asr::Transcript>, ApiError> {
    p.require("media:read")?;
    p.require("operations:create")?;
    p.require_account(&input.account_id)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let result = local_work(state.application, move |app| {
        app.transcribe_local(
            &input.account_id,
            &input.source,
            &input.model,
            &input.language,
            &request,
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    p.require("media:read")?;
    p.require_account(&account)?;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum RecordAction {
    List {
        account_id: String,
        category: String,
        before: Option<i64>,
    },
    Save {
        account_id: String,
        id: Option<String>,
        category: String,
        revision: i64,
        entry: werc_application::library::LibraryEntry,
    },
    Delete {
        account_id: String,
        id: String,
        category: String,
        revision: i64,
    },
}
pub(super) async fn records(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<RecordAction>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let account = match &input {
        RecordAction::List { account_id, .. }
        | RecordAction::Save { account_id, .. }
        | RecordAction::Delete { account_id, .. } => account_id.clone(),
    };
    p.require("sources:read")?;
    p.require_account(&account)?;
    if !matches!(&input, RecordAction::List { .. }) {
        p.require("operations:create")?;
    }
    let app = state.application.clone();
    let secret = bearer(&headers)?.to_owned();
    let output = local_work(state.application, move |app| match input {
        RecordAction::List {
            account_id,
            category,
            before,
        } => serde_json::to_value(app.library_page_local(&account_id, &category, before)?)
            .map_err(|_| AppError::internal()),
        RecordAction::Save {
            account_id,
            id,
            category,
            revision,
            entry,
        } => serde_json::to_value(app.library_save_local(
            &account_id,
            id.as_deref(),
            &category,
            revision,
            entry,
        )?)
        .map_err(|_| AppError::internal()),
        RecordAction::Delete {
            account_id,
            id,
            category,
            revision,
        } => {
            app.library_delete_local(&account_id, &id, &category, revision)?;
            Ok(json!({"deleted":true}))
        }
    })
    .await?;
    let p = app.authenticate(&secret)?;
    p.require("sources:read")?;
    p.require_account(&account)?;
    Ok(Json(output))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CaptureAction {
    LocateMessage { account_id:String,id:String,username:String,server_id:String,request_id:String },
    SocialReport { account_id:String,id:String,#[serde(default)] filter:werc_application::capture_tools::CaptureFilter,request_id:String },
    Timeline { account_id:String, id:String, #[serde(default)] query:werc_application::capture_timeline::TimelineQuery, after:Option<werc_application::capture_timeline::TimelineCursor>, request_id:String },
    Records {
        account_id: String,
        id: String,
        #[serde(default)]
        query: werc_application::source_capture::RecordQuery,
        after: Option<werc_application::source_capture::MediaPosition>,
        request_id: String,
    },
    MediaCatalog {
        account_id: String,
        id: String,
        #[serde(default)]
        query: werc_application::source_capture::MediaQuery,
        after: Option<werc_application::source_capture::MediaPosition>,
        request_id: String,
    },
    TranscriptProgress {
        account_id: String,
        id: String,
        model: String,
        language: String,
    },
    TranscriptStep {
        account_id: String,
        id: String,
        model: String,
        language: String,
        #[serde(default)]
        restart: bool,
        request_id: String,
    },
    MediaPage {
        account_id: String,
        id: String,
        index: u64,
    },
    MediaProgress {
        account_id: String,
        id: String,
    },
    MediaStep {
        account_id: String,
        id: String,
        #[serde(default)]
        restart: bool,
        request_id: String,
    },
    MediaSave {
        account_id: String,
        id: String,
        index: u64,
        row: usize,
        request_id: String,
    },
    Start {
        account_id: String,
        view_id: String,
        from: Option<i64>,
        until: Option<i64>,
    },
    Step {
        account_id: String,
        id: String,
        request_id: String,
    },
    List {
        account_id: String,
        before: Option<i64>,
    },
    Page {
        account_id: String,
        id: String,
        index: u64,
    },
}
impl CaptureAction {
    fn scopes(&self) -> &'static [&'static str] {
        match self {
            Self::SocialReport {..} => &["messages:read","moments:read"],
            Self::MediaCatalog { .. }
            | Self::MediaPage { .. }
            | Self::MediaProgress { .. }
            | Self::TranscriptProgress { .. } => &["messages:read", "media:read"],
            Self::MediaStep { .. } | Self::MediaSave { .. } => {
                &["messages:read", "media:write", "exports:create"]
            }
            Self::TranscriptStep { .. } => &["messages:read", "media:read", "operations:create"],
            Self::Start { .. } | Self::Step { .. } => &["messages:read", "exports:create"],
            _ => &["messages:read"],
        }
    }
    fn account(&self) -> &str {
        match self {
            Self::LocateMessage {account_id,..}
            | Self::SocialReport {account_id,..}
            | Self::Timeline { account_id, .. }
            | Self::Records { account_id, .. }
            | Self::MediaCatalog { account_id, .. }
            | Self::TranscriptProgress { account_id, .. }
            | Self::TranscriptStep { account_id, .. }
            | Self::MediaPage { account_id, .. }
            | Self::MediaProgress { account_id, .. }
            | Self::MediaStep { account_id, .. }
            | Self::MediaSave { account_id, .. }
            | Self::Start { account_id, .. }
            | Self::Step { account_id, .. }
            | Self::List { account_id, .. }
            | Self::Page { account_id, .. } => account_id,
        }
    }
}
pub(crate) fn capture_call(
    app: &Application,
    p: &Principal,
    input: CaptureAction,
) -> Result<serde_json::Value, AppError> {
    p.require("messages:read")?;
    p.require_account(input.account())?;
    let result = match input {
        CaptureAction::LocateMessage {account_id,id,username,server_id,request_id}=>{
            let request=conversation::request_key(p,&account_id,&request_id).map_err(|e|e.0)?;
            serde_json::to_value(app.capture_message_location_local(&account_id,&id,&username,&server_id,&request)?)
        }
        CaptureAction::SocialReport {account_id,id,filter,request_id}=>{
            p.require("moments:read")?;
            let request=conversation::request_key(p,&account_id,&request_id).map_err(|e|e.0)?;
            serde_json::to_value(app.capture_social_report_local(&account_id,&id,&filter,&request)?)
        }
        CaptureAction::Timeline {account_id,id,query,after,request_id} => {
            let request=conversation::request_key(p,&account_id,&request_id).map_err(|e|e.0)?;
            serde_json::to_value(app.capture_timeline_local(&account_id,&id,&query,after.as_ref(),&request)?)
        }
        CaptureAction::Records {
            account_id,
            id,
            query,
            after,
            request_id,
        } => {
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.capture_records_local(
                &account_id,
                &id,
                &query,
                after,
                &request,
            )?)
        }
        CaptureAction::MediaCatalog {
            account_id,
            id,
            query,
            after,
            request_id,
        } => {
            p.require("media:read")?;
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.capture_media_catalog_local(
                &account_id,
                &id,
                &query,
                after,
                &request,
            )?)
        }
        CaptureAction::TranscriptProgress {
            account_id,
            id,
            model,
            language,
        } => {
            p.require("media:read")?;
            serde_json::to_value(app.capture_transcript_progress_local(
                &account_id,
                &id,
                &model,
                &language,
            )?)
        }
        CaptureAction::TranscriptStep {
            account_id,
            id,
            model,
            language,
            restart,
            request_id,
        } => {
            p.require("media:read")?;
            p.require("operations:create")?;
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.step_capture_transcripts_local(
                &account_id,
                &id,
                &model,
                &language,
                restart,
                &request,
            )?)
        }
        CaptureAction::MediaPage {
            account_id,
            id,
            index,
        } => {
            p.require("media:read")?;
            serde_json::to_value(app.capture_media_page_local(&account_id, &id, index)?)
        }
        CaptureAction::MediaProgress { account_id, id } => {
            p.require("media:read")?;
            serde_json::to_value(app.capture_media_progress_local(&account_id, &id)?)
        }
        CaptureAction::MediaStep {
            account_id,
            id,
            restart,
            request_id,
        } => {
            p.require("media:write")?;
            p.require("exports:create")?;
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.step_capture_media_local(
                &account_id,
                &id,
                restart,
                &request,
            )?)
        }
        CaptureAction::MediaSave {
            account_id,
            id,
            index,
            row,
            request_id,
        } => {
            p.require("media:write")?;
            p.require("exports:create")?;
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.archive_capture_media_local(
                &account_id,
                &id,
                index,
                row,
                &request,
            )?)
        }
        CaptureAction::Start {
            account_id,
            view_id,
            from,
            until,
        } => {
            p.require("exports:create")?;
            serde_json::to_value(app.start_source_capture_local(
                &account_id,
                &view_id,
                from,
                until,
            )?)
        }
        CaptureAction::Step {
            account_id,
            id,
            request_id,
        } => {
            p.require("exports:create")?;
            let request =
                conversation::request_key(p, &account_id, &request_id).map_err(|e| e.0)?;
            serde_json::to_value(app.step_source_capture_local(&account_id, &id, &request)?)
        }
        CaptureAction::List { account_id, before } => {
            serde_json::to_value(app.source_captures_local(&account_id, before)?)
        }
        CaptureAction::Page {
            account_id,
            id,
            index,
        } => serde_json::to_value(app.source_capture_page_local(&account_id, &id, index)?),
    };
    result.map_err(|_| AppError::internal())
}
pub(super) async fn capture(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<CaptureAction>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account().to_owned();
    let scopes = input.scopes();
    for scope in scopes {
        p.require(scope)?;
    }
    p.require_account(&account)?;
    let auth = secret.clone();
    let result = local_work(state.application, move |app| {
        let current = app.authenticate(&auth)?;
        for scope in scopes {
            current.require(scope)?;
        }
        capture_call(app, &current, input)
    })
    .await?;
    let p = app.authenticate(&secret)?;
    for scope in scopes {
        p.require(scope)?;
    }
    p.require_account(&account)?;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureMediaContent {
    account_id: String,
    id: String,
    index: u64,
    row: usize,
    request_id: String,
}
pub(super) async fn capture_media_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<CaptureMediaContent>,
) -> Result<Response, ApiError> {
    fn access(p: &Principal, account: &str) -> Result<(), AppError> {
        p.require("messages:read")?;
        p.require("media:read")?;
        p.require_account(account)
    }
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id)?;
        app.capture_media_content_local(
            &input.account_id,
            &input.id,
            input.index,
            input.row,
            &request,
        )
    })
    .await?;
    access(&app.authenticate(&secret)?, &account)?;
    let name: String = output
        .file
        .file_name
        .bytes()
        .map(|b| format!("%{b:02X}"))
        .collect();
    Ok((
        [
            (header::CONTENT_TYPE, output.mime.to_owned()),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename*=UTF-8''{name}"),
            ),
        ],
        output.file.bytes.to_vec(),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordExportInput {
    account_id: String,
    id: String,
    #[serde(default)]
    query: werc_application::source_capture::RecordQuery,
    format: String,
    request_id: String,
}
pub(super) async fn record_export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<RecordExportInput>,
) -> Result<Response, ApiError> {
    fn access(p: &Principal, a: &str) -> Result<(), AppError> {
        p.require("messages:read")?;
        p.require("exports:create")?;
        p.require_account(a)
    }
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let mime = match input.format.as_str() {
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv; charset=utf-8",
        "json" => "application/json",
        "jsonl" => "application/x-ndjson",
        _ => return Err(AppError::new("INVALID_ARGUMENT", "记录导出格式无效").into()),
    };
    let bytes = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id)?;
        struct Bounded(zeroize::Zeroizing<Vec<u8>>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.0.len() + bytes.len() > 64 * 1024 * 1024 {
                    return Err(std::io::Error::other("export limit"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = Bounded(zeroize::Zeroizing::new(Vec::new()));
        app.export_capture_records_local(
            &input.account_id,
            &input.id,
            &input.query,
            &input.format,
            &request,
            &mut out,
        )?;
        Ok(out.0)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account)?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        bytes.to_vec(),
    )
        .into_response())
}
