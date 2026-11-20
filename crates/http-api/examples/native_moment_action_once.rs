//! New once-only like/comment on an existing, authenticated private automation post.
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
    previous: &Path,
    out: &Path,
) -> AppResult<serde_json::Value> {
    let selected = if previous == Path::new("-") {
        None
    } else {
        Some(old_post(previous, out)?)
    };
    let installations = discovery::discover()?.installations;
    let client = installations
        .iter()
        .find(|c| c.process_ids.contains(&pid))
        .ok_or_else(|| fail("CLIENT_MISSING"))?;
    let repo = Arc::new(SqliteRepository::open(&out.join("state.sqlite3"))?);
    let account = repo.register_account("自动化测试：本人私密动态互动")?.id;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &client.executable_path)?;
    provider.connect_pinned(&account, host, hash, pid, &client.executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        let current_id = provider
            .resolve_send_context(&account, "filehelper")?
            .sender_id;
        let (sns_id, self_id) = if let Some(value) = selected {
            value
        } else {
            let body = "自动化测试：WeRC 原生点赞与评论验证 2026-11-13。仅自己可见，仅发布一次。";
            let draft = app.create_private_moment_local(
                &account,
                body,
                "automation-interaction-private-post-20261113",
            )?;
            let preview = app.send_preview_local(&account, &draft.metadata.id)?;
            app.approve_text_send_local(
                &account,
                &draft.metadata.id,
                &draft.metadata.request_digest,
                true,
            )?;
            marker(out, "post-started")?;
            let mut status = app.execute_send_local(&account, &draft.metadata.id)?;
            let until = Instant::now() + Duration::from_secs(40);
            while status.state == SendState::Unknown && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(300));
                status = app.reconcile_send_local(&account, &draft.metadata.id)?;
            }
            if status.state != SendState::ServerAccepted {
                return Err(fail("POST_UNCONFIRMED_NO_RETRY"));
            }
            let receipt = provider.reconcile_private_moment(
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
            let id = receipt
                .provider_reference
                .as_deref()
                .and_then(|s| s.strip_prefix("native-moment:"))
                .ok_or_else(|| fail("POST_ID_MISSING"))?
                .to_owned();
            (id, current_id.clone())
        };
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
        let mut tasks = vec![];
        for kind in [MomentActionKind::Like, MomentActionKind::Comment] {
            let (text, key, name) = match kind {
                MomentActionKind::Like => {
                    ("", "automation-self-like-once-20261113", "like-started")
                }
                MomentActionKind::Comment => (
                    "自动化测试：WeRC 原生评论验证 2026-11-13（仅一次）。",
                    "automation-self-comment-once-20261113",
                    "comment-started",
                ),
                _ => return Err(fail("UNEXPECTED_ACTION")),
            };
            let action = MomentAction {
                reply: None,
                sns_id: sns_id.clone(),
                action: kind,
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
        serde_json::json!({"passed":tasks.len()==2&&tasks.iter().all(|t|t["native_state"]==11),"tasks":tasks,"encrypted_reopen":true,"owned_private_automation_post_only":true,"new_posts":usize::from(out.join("post-started").exists()),"server_confirmation":false,"normal_product_store":false,"automatic_retry":false}),
    )
}
fn main() {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.len() != 6
        || !matches!(
            a[0].as_str(),
            "--consent-once-own-automation-post-like-comment"
                | "--consent-once-new-private-automation-post-like-comment"
        )
    {
        std::process::exit(2)
    }
    if a[4] == "-" && a[0] != "--consent-once-new-private-automation-post-like-comment" {
        std::process::exit(2)
    }
    let out = Path::new(&a[5]);
    std::fs::create_dir(out).expect("fixed once directory exists; do not replay");
    let value=run(a[1].parse().unwrap(),Path::new(&a[2]),&a[3],Path::new(&a[4]),out).unwrap_or_else(|e|serde_json::json!({"passed":false,"error":e.code,"post_started":out.join("post-started").exists(),"like_started":out.join("like-started").exists(),"comment_started":out.join("comment-started").exists(),"automatic_retry":false}));
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
