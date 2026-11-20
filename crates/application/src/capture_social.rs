//! Join exact contact identities across the selected chat archive and saved SNS.
use crate::{
    Application,
    capture_tools::{AnalysisView, CaptureFilter},
    moment_preservation::PreservedMoment,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use werc_domain::{AppError, AppResult, live::valid_live_username, moments::MomentInteraction};

#[derive(Default, Serialize)]
pub struct SocialContact {
    pub username: String,
    pub messages: u64,
    pub sent: u64,
    pub received: u64,
    pub posts: u64,
    pub likes_given: u64,
    pub likes_received: u64,
    pub comments_given: u64,
    pub comments_received: u64,
}
#[derive(Serialize)]
pub struct CaptureSocialReport {
    pub generated_at: String,
    pub filter: CaptureFilter,
    pub self_username: String,
    pub contacts: Vec<SocialContact>,
    pub own_posts: u64,
    pub observed_posts: u64,
    pub unreadable_posts: u64,
    pub unmatched_login_posts: u64,
    pub interactions_without_time: u64,
    pub complete_selected_sources: bool,
    pub source_mode: &'static str,
}
fn within(timestamp: i64, filter: &CaptureFilter) -> bool {
    filter.from.is_none_or(|a| timestamp >= a) && filter.until.is_none_or(|b| timestamp < b)
}
fn included(username: &str, login: &str, filter: &CaptureFilter) -> bool {
    username != login
        && valid_live_username(username)
        && werc_domain::live::is_private_account(username)
        && !filter.exclude_conversations.iter().any(|s| s == username)
        && (filter.conversations.is_empty() || filter.conversations.iter().any(|s| s == username))
        && matches!(filter.view, AnalysisView::All | AnalysisView::Private)
}
fn row<'a>(rows: &'a mut BTreeMap<String, SocialContact>, username: &str) -> &'a mut SocialContact {
    rows.entry(username.into())
        .or_insert_with(|| SocialContact {
            username: username.into(),
            ..Default::default()
        })
}
fn interaction_time(item: &MomentInteraction, filter: &CaptureFilter, missing: &mut u64) -> bool {
    match item.timestamp {
        Some(time) => within(time, filter),
        None => {
            *missing += 1;
            filter.from.is_none() && filter.until.is_none()
        }
    }
}
impl Application {
    pub fn capture_social_report_local(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        request_id: &str,
    ) -> AppResult<CaptureSocialReport> {
        let chat_snapshot = {
            let lease = self.browse_lease(request_id)?.for_local_archive();
            self.capture_revision_snapshot(account, id, || lease.check())?
        };
        let messages = self.capture_report_local(account, id, filter, request_id)?;
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let login = &messages.self_username;
        if login.is_empty() {
            return Err(AppError::new(
                "IDENTITY_UNAVAILABLE",
                "归档未保存登录账号，无法关联朋友圈身份",
            ));
        }
        let mut contacts = BTreeMap::new();
        for (name, s) in &messages.conversations {
            if included(name, login, filter) {
                let r = row(&mut contacts, name);
                r.messages = s.total;
                r.sent = s.sent;
                r.received = s.received;
            }
        }
        let mut result = CaptureSocialReport {
            generated_at: messages.generated_at,
            filter: filter.clone(),
            self_username: login.clone(),
            contacts: vec![],
            own_posts: 0,
            observed_posts: 0,
            unreadable_posts: 0,
            unmatched_login_posts: 0,
            interactions_without_time: 0,
            complete_selected_sources: messages.complete_selected_sources,
            source_mode: "saved_messages_and_preserved_moments",
        };
        let mut cursor = None;
        let mut revisions = Vec::new();
        let mut seen = BTreeSet::new();
        loop {
            lease.check()?;
            let page = self
                .repository
                .library_list(account, "moment_preserved", cursor, 100)?;
            for header in page.data {
                lease.check()?;
                let (current, item) =
                    self.open_document::<PreservedMoment>(account, &header.id, "moment_preserved")?;
                revisions.push((header.id.clone(), current.revision));
                // New copies retain login for portable backup restore. Older
                // copies can be matched by their authenticated exact owner ID.
                let owner_matches = if let Some(owner) = &item.login {
                    owner == login
                } else {
                    header.id
                        == format!(
                            "moment-{}",
                            self.send_digest(
                                account,
                                "preserved-moment-id",
                                &(login, &item.moment.anchor.id, &item.moment.anchor.username)
                            )?
                        )
                };
                if !owner_matches {
                    result.unmatched_login_posts += 1;
                    continue;
                }
                let moment = &item.moment;
                if !seen.insert((moment.anchor.username.clone(), moment.anchor.id.clone())) {
                    continue;
                }
                let Some(content) = &moment.content else {
                    result.unreadable_posts += 1;
                    continue;
                };
                let own = moment.anchor.username == *login;
                if matches!(filter.view,AnalysisView::All|AnalysisView::Private) && within(content.timestamp, filter)
                    && (own || included(&moment.anchor.username, login, filter))
                {
                    result.observed_posts += 1;
                    if own {
                        result.own_posts += 1;
                    } else {
                        row(&mut contacts, &moment.anchor.username).posts += 1;
                    }
                }
                let mut likers = BTreeSet::new();
                for interaction in &content.details.likes {
                    if interaction.deleted == Some(true)
                        || !likers.insert(interaction.username.as_str())
                    {
                        continue;
                    }
                    if !interaction_time(interaction, filter, &mut result.interactions_without_time)
                    {
                        continue;
                    }
                    if own && included(&interaction.username, login, filter) {
                        row(&mut contacts, &interaction.username).likes_received += 1;
                    } else if interaction.username == *login
                        && included(&moment.anchor.username, login, filter)
                    {
                        row(&mut contacts, &moment.anchor.username).likes_given += 1;
                    }
                }
                let mut comments = BTreeSet::new();
                for interaction in &content.details.comments {
                    if interaction.deleted == Some(true) {
                        continue;
                    }
                    let identity = serde_json::to_string(&(
                        &interaction.username,
                        &interaction.id64,
                        &interaction.id,
                        &interaction.timestamp,
                        &interaction.text,
                        &interaction.reply_username,
                    ))
                    .map_err(|_| AppError::internal())?;
                    if !comments.insert(identity)
                        || !interaction_time(
                            interaction,
                            filter,
                            &mut result.interactions_without_time,
                        )
                    {
                        continue;
                    }
                    // A reply to a third party on my post is not a direct
                    // comment to me. Unknown/empty reply targets mean the post.
                    if own
                        && (interaction.reply_username.is_empty()
                            || interaction.reply_username == *login)
                        && included(&interaction.username, login, filter)
                    {
                        row(&mut contacts, &interaction.username).comments_received += 1;
                    } else if interaction.username == *login {
                        let target = if interaction.reply_username.is_empty() {
                            &moment.anchor.username
                        } else {
                            &interaction.reply_username
                        };
                        if included(target, login, filter) {
                            row(&mut contacts, target).comments_given += 1;
                        }
                    }
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        for (id, revision) in revisions {
            lease.check()?;
            if self.repository.library_get(account, &id)?.header.revision != revision {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "分析期间朋友圈副本已变化，请重新生成",
                ));
            }
        }
        if self.capture_revision_snapshot(account, id, || lease.check())? != chat_snapshot {
            return Err(AppError::new(
                "DOCUMENT_CONFLICT",
                "分析期间聊天副本已变化，请重新生成",
            ));
        }
        result.contacts = contacts.into_values().collect();
        result.contacts.sort_by(|a, b| {
            b.messages
                .cmp(&a.messages)
                .then(
                    (b.likes_given + b.likes_received + b.comments_given + b.comments_received)
                        .cmp(
                            &(a.likes_given
                                + a.likes_received
                                + a.comments_given
                                + a.comments_received),
                        ),
                )
                .then(a.username.cmp(&b.username))
        });
        Ok(result)
    }
    pub fn export_capture_social_local(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        format: &str,
        request_id: &str,
    ) -> AppResult<Vec<u8>> {
        if !matches!(format, "json" | "txt" | "html") {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "关联报告支持JSON、TXT或HTML",
            ));
        }
        let report = self.capture_social_report_local(account, id, filter, request_id)?;
        if format == "json" {
            return serde_json::to_vec_pretty(&report).map_err(|_| AppError::internal());
        }
        let mut text = format!(
            "WeRC 聊天与朋友圈关联报告\n生成时间：{}\n仅已保存本地记录；不是完整历史或实时社交关系。\n本人动态 {} / 所选作者动态 {}\n无时间互动 {}（指定日期时不计入）/ 登录身份不匹配 {} / 无法读取动态 {}\n",
            report.generated_at,
            report.own_posts,
            report.observed_posts,
            report.interactions_without_time,
            report.unmatched_login_posts,
            report.unreadable_posts
        );
        for r in &report.contacts {
            text.push_str(&format!("\n{}\n聊天 {}（我 {} / 对方 {}）· 动态 {} · 我点赞 {} / 收赞 {} · 我评论 {} / 收评论 {}\n",r.username,r.messages,r.sent,r.received,r.posts,r.likes_given,r.likes_received,r.comments_given,r.comments_received));
        }
        if format == "html" {
            text = format!(
                "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>WeRC 关联报告</title><style>body{{font:16px system-ui;margin:2rem;line-height:1.6}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}</style><pre>{}</pre></html>",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            );
        }
        Ok(text.into_bytes())
    }
}
