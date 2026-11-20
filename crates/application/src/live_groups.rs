use crate::{Application, live_sessions::ParentView};
use serde::Serialize;
use werc_domain::{AppError, AppResult, group::*};
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "本地群成员来源或窗口已变化，请重新读取")
}
impl ParentView<'_> {
    fn open_group(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<LiveGroupContinuation> {
        match self {
            Self::Single(v) => v.open_group(binding, id, cursor),
            Self::Multi(v) => v.open_group(binding, id, cursor),
        }
    }
    fn seal_group(
        &self,
        binding: &str,
        id: &str,
        value: &LiveGroupContinuation,
    ) -> AppResult<String> {
        match self {
            Self::Single(v) => v.seal_group(binding, id, value),
            Self::Multi(v) => v.seal_group(binding, id, value),
        }
    }
}
#[derive(Serialize)]
pub struct LiveGroupPage {
    pub username: String,
    pub owner_username: Option<String>,
    pub cached_pins: Option<Vec<LiveGroupPin>>,
    pub rows: Vec<LiveGroupMember>,
    pub next: Option<String>,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub complete_server_membership: bool,
    pub cross_page_snapshot: bool,
}
impl Application {
    pub fn live_group_members_local(
        &self,
        account: &str,
        view_id: &str,
        username: &str,
        cursor: Option<&str>,
        request_id: &str,
    ) -> AppResult<LiveGroupPage> {
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        self.read_groups_with_lease(account, view_id, username, cursor, &lease)
    }
    pub(crate) fn read_groups_with_lease(
        &self,
        account: &str,
        view_id: &str,
        username: &str,
        cursor: Option<&str>,
        lease: &crate::history::QueryLease<'_>,
    ) -> AppResult<LiveGroupPage> {
        if !valid_group_username(username) {
            return Err(AppError::new(
                "LIVE_GROUP_SELECTION_INVALID",
                "请选择一个群会话",
            ));
        }
        lease.check()?;
        let (database, binding, source, after) =
            self.with_live_source_parent(account, view_id, |database, binding, source, view| {
                let after = cursor
                    .map(|c| view.open_group(binding, view_id, c))
                    .transpose()?;
                if after
                    .as_ref()
                    .is_some_and(|a| a.selection.username != username)
                {
                    return Err(stale());
                }
                Ok((
                    zeroize::Zeroizing::new(database.to_owned()),
                    binding.to_owned(),
                    source.to_owned(),
                    after,
                ))
            })?;
        let selection = after
            .as_ref()
            .map(|a| a.selection.clone())
            .unwrap_or(LiveGroupSelection {
                username: username.into(),
                after: None,
            });
        let (fingerprint, mut window) = self.provider.read_live_group_members(
            account,
            &database,
            &source,
            &selection,
            after.as_ref().map(|a| a.source_fingerprint.as_str()),
            &lease.cancel,
        )?;
        lease.check()?;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || after
                .as_ref()
                .is_some_and(|a| a.source_fingerprint != fingerprint)
            || !window.valid(&selection)
        {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "群成员分页来源或结果无效",
            ));
        }
        let plain =
            zeroize::Zeroizing::new(serde_json::to_vec(&window).map_err(|_| AppError::internal())?);
        if plain.len() > LIVE_GROUP_MAX_BYTES {
            return Err(AppError::new(
                "LIVE_GROUP_WINDOW_LIMIT",
                "群成员页超过读取上限",
            ));
        }
        self.with_live_source_parent(
            account,
            view_id,
            |current_database, current_binding, current_source, view| {
                if current_database != database.as_str()
                    || current_binding != binding
                    || current_source != source
                {
                    return Err(stale());
                }
                let expires_in_seconds = view.check(&binding, view_id)?;
                lease.check()?;
                let next = if window.has_more {
                    Some(view.seal_group(
                        &binding,
                        view_id,
                        &LiveGroupContinuation {
                            source_fingerprint: fingerprint,
                            selection: LiveGroupSelection {
                                username: username.into(),
                                after: Some(window.rows.last().ok_or_else(stale)?.anchor.clone()),
                            },
                        },
                    )?)
                } else {
                    None
                };
                Ok(LiveGroupPage {
                    username: std::mem::take(&mut window.username),
                    owner_username: window.owner_username.take(),
                    cached_pins: window.cached_pins.take(),
                    rows: std::mem::take(&mut window.rows),
                    next,
                    expires_in_seconds,
                    mode: "local_group_cache",
                    complete_server_membership: false,
                    cross_page_snapshot: false,
                })
            },
        )
    }
}
