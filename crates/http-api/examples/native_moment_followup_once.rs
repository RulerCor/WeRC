//! Once-only reply/unlike/delete of the authenticated, previously published automation post.
use std::{
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, outbox::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{
    WindowsWechatProvider, discovery, managed_messages::ManagedWindowsWechatProvider,
};
fn fail(code: &str) -> AppError {
    AppError::new(code, "自动化测试未通过；不重发")
}
fn marker(dir: &Path, name: &str) -> AppResult<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(name))
        .map_err(|_| fail("ATTEMPT_ALREADY_STARTED"))?;
    f.write_all(b"automatic test; never replay")
        .map_err(|_| fail("OUTPUT"))?;
    f.sync_all().map_err(|_| fail("OUTPUT"))
}
fn old_post(previous: &Path, out: &Path) -> AppResult<(String, String)> {
    // Work on a copy of the old isolated verification store. Never invoke its old task.
    std::fs::copy(previous, out.join("previous-copy.sqlite3")).map_err(|_| fail("COPY_FAILED"))?;
    let old = Arc::new(SqliteRepository::open(&out.join("previous-copy.sqlite3"))?);
    let crypto = Arc::new(WindowsWechatProvider);
    let offline = Application::new(old.clone(), crypto.clone());
    let mut selected = None;
    for account in old.accounts()? {
        for task in old.sends(&account.id)? {
            if task.metadata.schema_version != 9 || task.state != SendState::ServerAccepted {
                continue;
            }
            let p = offline.send_preview_local(&account.id, &task.metadata.id)?;
            if !p.payload.text.contains("自动化测试")
                || p.payload.context.sender_id != p.payload.context.target_id
                || p.payload.moment.as_ref().is_none_or(|m| {
                    m.visibility != MomentVisibility::Private || m.interaction.is_some()
                })
            {
                continue;
            }
            let wrapped = old
                .send_receipt(&account.id, &task.metadata.id)?
                .ok_or_else(|| fail("OLD_PROOF_MISSING"))?;
            let raw = zeroize::Zeroizing::new(crypto.unprotect_key(
                &format!("WeRC/outbox-receipt/v1/{}/{}", account.id, task.metadata.id),
                &wrapped,
            )?);
            let receipt: SendReceipt =
                serde_json::from_slice(&raw).map_err(|_| fail("OLD_PROOF_INVALID"))?;
            if receipt.outbox_id != task.metadata.id
                || Some(receipt.attempt_id) != task.attempt_id
                || receipt.context_digest != task.metadata.context_digest
                || receipt.evidence != SendEvidence::ServerAccepted
            {
                return Err(fail("OLD_PROOF_INVALID"));
            }
            let id = receipt
                .provider_reference
                .as_deref()
                .and_then(|s| {
                    s.strip_prefix("native-moment:")
                        .or_else(|| s.strip_prefix("native-private-moment:"))
                })
                .ok_or_else(|| fail("OLD_PROOF_INVALID"))?;
            if id.parse::<u64>().ok().filter(|n| *n > 0).is_none() {
                return Err(fail("OLD_PROOF_INVALID"));
            }
            if selected.is_some() {
                return Err(fail("AMBIGUOUS_OWNED_POST"));
            }
            selected = Some((id.to_owned(), p.payload.context.sender_id.clone()));
        }
    }
    let result = selected.ok_or_else(|| fail("OWNED_TEST_POST_NOT_FOUND"))?;
    drop(offline);
    drop(old);
    Ok(result)
}
fn run(
    pid: u32,
    host: &Path,
    hash: &str,
    worker: &Path,
    worker_hash: &str,
    previous: &Path,
    out: &Path,
    cleanup_only: bool,
) -> AppResult<serde_json::Value> {
    let selected = old_post(previous, out)?;
    let installations = discovery::discover()?.installations;
    let client = installations
        .iter()
        .find(|c| c.process_ids.contains(&pid))
        .ok_or_else(|| fail("CLIENT_MISSING"))?;
    let repo = Arc::new(SqliteRepository::open(&out.join("state.sqlite3"))?);
    let account = repo.register_account("自动化测试：本人私密动态互动")?.id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_owned(), worker_hash.into())
            .with_live_window_limit(120)?,
    );
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &client.executable_path)?;
    provider.connect_pinned(&account, host, hash, pid, &client.executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        let current_id = provider
            .resolve_send_context(&account, "filehelper")?
            .sender_id;
        let (sns_id, self_id) = selected;
        if current_id != self_id {
            return Err(fail("ACCOUNT_CHANGED"));
        }
        let lookup = provider.probe_moment_action(&sns_id)?;
        if lookup != (22, 5) {
            return Err(fail(&format!(
                "OWNED_POST_LOOKUP_{}_{}",
                lookup.0, lookup.1
            )));
        }
        // Allow the final native task destructor to retire before submitting next work.
        std::thread::sleep(Duration::from_millis(750));
        let catalog = werc_wechat_win::data_sources::discover()?;
        if catalog.sources.len() != 1 {
            return Err(fail("SOURCE_SELECTION_REQUIRED"));
        }
        let source = &catalog.sources[0];
        let database = source
            .databases
            .iter()
            .filter(|d| {
                d.wal_present
                    && d.relative_path
                        .strip_prefix("db_storage/message/message_")
                        .and_then(|s| s.strip_suffix(".db"))
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            })
            .max_by_key(|d| &d.relative_path)
            .ok_or_else(|| fail("SOURCE_MISSING"))?;
        let path = Path::new(&source.account_directory).join(&database.relative_path);
        let path = path.to_str().ok_or_else(|| fail("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, path, true)?;
        app.acquire_database_key_local(&account, pid, path, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, path, "moment-actions-source", true)?;
        let mut cursor = None;
        let mut reply = None;
        let mut self_like = false;
        for _ in 0..10 {
            let page = app.query_live_moments_local(
                &account,
                &view.view_id,
                None,
                cursor.as_deref(),
                "moment-actions-read",
            )?;
            if let Some(post) = page
                .rows
                .iter()
                .find(|r| r.anchor.id == sns_id && r.anchor.username == self_id)
            {
                let content = post
                    .content
                    .as_ref()
                    .ok_or_else(|| fail("POST_UNREADABLE"))?;
                if !content.text.contains("自动化测试") {
                    return Err(fail("WRONG_POST"));
                }
                self_like = content.details.likes.iter().any(|v| v.username == self_id);
                let comments: Vec<_> = content
                    .details
                    .comments
                    .iter()
                    .filter(|c| {
                        c.username == self_id
                            && c.text == "自动化测试：WeRC 原生评论验证 2026-11-13（仅一次）。"
                            && c.deleted != Some(true)
                    })
                    .collect();
                if comments.len() != 1 {
                    return Err(fail("EXACT_AUTOMATION_COMMENT_NOT_UNIQUE"));
                }
                let c = comments[0];
                let target = MomentReplyTarget {
                    username: c.username.clone(),
                    id: c.id.clone().filter(|v| v != "0"),
                    id64: c.id64.clone().filter(|v| v != "0"),
                };
                if !target.valid() {
                    return Err(fail("COMMENT_ID_MISSING"));
                }
                reply = Some(target);
                break;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let reply = reply.ok_or_else(|| fail("OWNED_POST_NOT_IN_CACHE"))?;
        std::fs::write(out.join("readback-before.json"),serde_json::to_vec_pretty(&serde_json::json!({"own_post_exact_id":true,"own_prior_comment_unique":true,"self_like_present":self_like,"reply_has_id32":reply.id.is_some(),"reply_has_id64":reply.id64.is_some()})).unwrap()).map_err(|_|fail("OUTPUT"))?;
        let mut tasks = vec![];
        for kind in [
            MomentActionKind::Reply,
            MomentActionKind::Unlike,
            MomentActionKind::Delete,
        ] {
            if cleanup_only && kind == MomentActionKind::Reply {
                continue;
            }
            let (text, key, name) = match kind {
                MomentActionKind::Reply => (
                    "自动化测试：WeRC 精确回复评论验证 2026-11-13（仅一次）。",
                    "automation-self-reply-once-20261113",
                    "reply-started",
                ),
                MomentActionKind::Unlike => {
                    ("", "automation-self-unlike-once-20261113", "unlike-started")
                }
                MomentActionKind::Delete => {
                    ("", "automation-self-delete-once-20261113", "delete-started")
                }
                _ => return Err(fail("UNEXPECTED_ACTION")),
            };
            let action = MomentAction {
                sns_id: sns_id.clone(),
                action: kind,
                reply: (kind == MomentActionKind::Reply).then(|| reply.clone()),
            };
            let draft = app.create_moment_action_local(&account, &action, text, key)?;
            let preview = app.send_preview_local(&account, &draft.metadata.id)?;
            if preview
                .payload
                .moment
                .as_ref()
                .and_then(|m| m.interaction.as_ref())
                != Some(&action)
                || preview.payload.text != text
            {
                return Err(fail("PREVIEW_CHANGED"));
            }
            app.approve_text_send_local(
                &account,
                &draft.metadata.id,
                &draft.metadata.request_digest,
                true,
            )?;
            marker(out, name)?;
            let mut status = app.execute_send_local(&account, &draft.metadata.id)?;
            let until = Instant::now() + Duration::from_secs(35);
            let mut native = None;
            loop {
                let receipt = provider.reconcile_moment_action(
                    TextReceiptQuery {
                        outbox_id: &draft.metadata.id,
                        attempt_id: status
                            .attempt_id
                            .as_deref()
                            .ok_or_else(|| fail("ATTEMPT_MISSING"))?,
                        context_digest: &draft.metadata.context_digest,
                        context: &preview.payload.context,
                    },
                    None,
                )?;
                let state = receipt
                    .provider_reference
                    .as_deref()
                    .and_then(|r| r.strip_prefix("native-moment-action-state:"))
                    .and_then(|n| n.parse::<u32>().ok());
                if matches!(state, Some(6..=9) | Some(11..=13)) {
                    native = state;
                    break;
                }
                if Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            status = app.reconcile_send_local(&account, &draft.metadata.id)?;
            if app.execute_send_local(&account, &draft.metadata.id).is_ok() {
                return Err(fail("RETRY_NOT_REFUSED"));
            }
            let row = serde_json::json!({"kind":kind,"native_state":native,"outbox_state":status.state,"attempts":1,"resubmission_refused":true,"task_id":draft.metadata.id});
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join(format!("{name}.json")))
                .map_err(|_| fail("OUTPUT"))?;
            f.write_all(&serde_json::to_vec_pretty(&row).unwrap())
                .map_err(|_| fail("OUTPUT"))?;
            f.sync_all().map_err(|_| fail("OUTPUT"))?;
            tasks.push(row);
            if native != Some(11) {
                break;
            }
            std::thread::sleep(Duration::from_millis(750));
        }
        Ok((sns_id, tasks))
    })();
    provider.disconnect(None)?;
    let (sns_id, tasks) = result?;
    let reopened = Arc::new(SqliteRepository::open(&out.join("state.sqlite3"))?);
    let offline = Application::new(reopened.clone(), provider);
    for task in &tasks {
        let id = task["task_id"].as_str().unwrap();
        let preview = offline.send_preview_local(&account, id)?;
        if preview
            .payload
            .moment
            .as_ref()
            .and_then(|m| m.interaction.as_ref())
            .is_none_or(|a| a.sns_id != sns_id)
            || reopened.send_receipt(&account, id)?.is_none()
        {
            return Err(fail("REOPEN_MISMATCH"));
        }
    }
    Ok(
        serde_json::json!({"passed":tasks.len()==if cleanup_only {2}else{3}&&tasks.iter().all(|t|t["native_state"]==11),"tasks":tasks,"encrypted_reopen":true,"owned_private_automation_post_only":true,"new_posts":0,"server_confirmation":false,"normal_product_store":false,"automatic_retry":false}),
    )
}
fn main() {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.len() != 8
        || !matches!(
            a[0].as_str(),
            "--consent-once-own-automation-post-reply-unlike-delete"
                | "--consent-once-own-automation-post-unlike-delete"
        )
    {
        std::process::exit(2)
    }
    let out = Path::new(&a[7]);
    std::fs::create_dir(out).expect("once directory exists; never replay");
    let value=run(a[1].parse().unwrap(),Path::new(&a[2]),&a[3],Path::new(&a[4]),&a[5],Path::new(&a[6]),out,a[0]=="--consent-once-own-automation-post-unlike-delete").unwrap_or_else(|e|serde_json::json!({"passed":false,"error":e.code,"reply_started":out.join("reply-started").exists(),"unlike_started":out.join("unlike-started").exists(),"delete_started":out.join("delete-started").exists(),"automatic_retry":false}));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("evidence.json"))
        .unwrap();
    f.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    f.sync_all().unwrap();
    println!("{value}");
    if value["passed"] != true {
        std::process::exit(1)
    }
}
