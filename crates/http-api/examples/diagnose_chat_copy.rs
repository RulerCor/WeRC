use std::{path::PathBuf, sync::Arc};
use werc_application::{Application, ports::Repository};
use werc_domain::{browse::*, history::HistoryQuery};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

// 只接受驱动创建的临时加密副本；不能用此入口迁移正常资料库。
fn run() -> Result<serde_json::Value, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--consent-protected-copy" {
        return Err("CONSENT_REQUIRED".into());
    }
    let path = PathBuf::from(&args[1]);
    let parent = path.parent().ok_or("COPY_REQUIRED")?;
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|_| "TEMP_UNAVAILABLE")?;
    if path.file_name().is_none_or(|name| name != "state.sqlite3")
        || parent.parent().and_then(|p| p.canonicalize().ok()).as_ref() != Some(&temp)
        || !parent
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("werc-chat-copy-"))
        || std::fs::read(parent.join(".werc-protected-copy")).map_err(|_| "COPY_MARKER_REQUIRED")?
            != b"WeRC encrypted state diagnostic copy v1\n"
    {
        return Err("COPY_REQUIRED".into());
    }
    for item in path.ancestors() {
        let metadata = std::fs::symlink_metadata(item).map_err(|_| "UNSAFE_COPY_PATH")?;
        if metadata.file_type().is_symlink() {
            return Err("UNSAFE_COPY_PATH".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("UNSAFE_COPY_PATH".into());
            }
        }
    }
    let repo = Arc::new(SqliteRepository::open(&path).map_err(|e| e.code)?);
    let current = repo.current_connection().map_err(|e| e.code)?;
    let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
    let mut candidates = Vec::new();
    for account in repo.accounts().map_err(|e| e.code)? {
        let sources = app.history_sources_local(&account.id).map_err(|e| e.code)?;
        if !sources.is_empty() {
            candidates.push((account.id, sources));
        }
    }
    if candidates.is_empty() {
        return Ok(
            serde_json::json!({"outcome":"no_imported_history","archive_scopes":0,"normal_state_migrated":false,"wechat_sources_opened":false,"messages_sent":false}),
        );
    }
    let index = match candidates
        .iter()
        .position(|(id, _)| current.account_id.as_ref() == Some(id))
    {
        Some(index) => index,
        None if candidates.len() == 1 => 0,
        None => return Err("AMBIGUOUS_ARCHIVE_SCOPE".into()),
    };
    let archive_scopes = candidates.len();
    let (account, sources) = candidates.swap_remove(index);
    let mut request = BrowseRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        mode: ReadMode::ImportedSnapshot,
        source_id: None,
        direction: BrowseDirection::Latest,
        anchor_id: None,
        anchor_timestamp: None,
        query: HistoryQuery {
            conversation_id: None,
            sender_id: None,
            message_type: None,
            search: None,
            from: None,
            until: None,
            cursor: None,
            limit: 60,
        },
    };
    let conversations = app
        .browse_conversations_local(&account, &request)
        .map_err(|e| e.code)?;
    let mut pages = Vec::new();
    // 私密身份/正文仅在进程内用于查询，绝不输出到诊断结果。
    for conversation in conversations.data.iter().take(3) {
        request.query.conversation_id = Some(conversation.id.clone());
        request.request_id = uuid::Uuid::new_v4().to_string();
        let page = app
            .browse_messages_local(&account, &request)
            .map_err(|e| e.code)?;
        let mut older_count = None;
        if let Some(cursor) = &page.older_cursor {
            let mut older = request.clone();
            older.request_id = uuid::Uuid::new_v4().to_string();
            older.direction = BrowseDirection::Older;
            older.query.cursor = Some(cursor.clone());
            let prior = app
                .browse_messages_local(&account, &older)
                .map_err(|e| e.code)?;
            if prior
                .data
                .iter()
                .any(|a| page.data.iter().any(|b| a.id == b.id))
            {
                return Err("PAGE_OVERLAP".into());
            }
            older_count = Some(prior.data.len());
        }
        pages.push(serde_json::json!({"latest_count":page.data.len(),"older_count":older_count,"query_strategy":page.context.query_strategy,"query_metrics":page.context.query_metrics,"identity_status":page.context.identity_status}));
    }
    Ok(serde_json::json!({
        "outcome":"protected_archive_browse_verified", "archive_scopes":archive_scopes,
        "imported_sources":sources.len(), "source_message_counts_sum_not_deduplicated":sources.iter().map(|source| source.message_count).sum::<usize>(),
        "conversation_first_page_count":conversations.data.len(), "conversation_query_metrics":conversations.context.query_metrics,
        "pages":pages, "selection_required":current.selection_required, "identity_verified":current.identity_verified,
        "normal_state_migrated":false, "copy_only":true, "wechat_sources_opened":false,
        "messages_sent":false, "real_online_wechat_verified":false
    }))
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(code) => {
            println!(
                "{}",
                serde_json::json!({"outcome":"failed","error_code":code,"normal_state_migrated":false,"messages_sent":false})
            );
            std::process::exit(1);
        }
    }
}
