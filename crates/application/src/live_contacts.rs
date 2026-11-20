use crate::{Application, live_sessions::ParentView};
use serde::Serialize;
use werc_domain::{AppError, AppResult, live::*};
#[derive(Serialize)]
pub struct CurrentProfile {pub username:String,pub nickname:Option<String>,pub avatar_available:bool}

#[derive(Serialize)]
pub struct DirectoryFriend {
    pub username: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
}

#[derive(Serialize)]
pub struct DirectoryContact {
    #[serde(flatten)]
    pub contact: DirectoryFriend,
    pub category: &'static str,
}

#[derive(Serialize)]
pub struct LiveContactPage {
    pub search_query: Option<String>,
    pub rows: Vec<LiveContact>,
    pub next: Option<String>,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub complete_address_book: bool,
    pub cross_page_snapshot: bool,
}

impl ParentView<'_> {
    fn open_contacts(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<LiveContactContinuation> {
        match self {
            Self::Single(v) => v.open_contacts(binding, id, cursor),
            Self::Multi(v) => v.open_contacts(binding, id, cursor),
        }
    }
    fn seal_contacts(
        &self,
        binding: &str,
        id: &str,
        value: &LiveContactContinuation,
    ) -> AppResult<String> {
        match self {
            Self::Single(v) => v.seal_contacts(binding, id, value),
            Self::Multi(v) => v.seal_contacts(binding, id, value),
        }
    }
}
impl Application {
    pub fn current_profile_local(&self,account:&str,view:&str,request:&str)->AppResult<CurrentProfile>{
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        let(context,_)=self.checked_send_context(account,"filehelper")?;
        let mut profile=CurrentProfile{username:context.sender_id.clone(),nickname:None,avatar_available:false};
        let mut cursor=None;
        loop{
            let page=self.read_contacts_with_lease(account,view,cursor.as_deref(),Some(&profile.username),&lease)?;
            if let Some(row)=page.rows.iter().find(|r|r.anchor.username==profile.username){profile.nickname=row.nickname.clone();profile.avatar_available=row.avatar_url.is_some();break;}
            cursor=page.next;if cursor.is_none(){break;}
        }
        lease.check()?;
        if self.checked_send_context(account,"filehelper")?.0!=context{return Err(AppError::new("CONNECTION_CHANGED","读取资料期间微信身份已变化"));}
        Ok(profile)
    }
    /// Read group names from the already-bound account root, without opening a history view.
    pub fn group_names_local(
        &self,
        account: &str,
        request_id: &str,
    ) -> AppResult<std::collections::BTreeMap<String, String>> {
        const QUERY: &str = "@chatroom";
        let mut names = std::collections::BTreeMap::new();
        self.visit_bound_contacts(account, request_id, Some(QUERY), |row| {
            let id = &row.anchor.username;
            if !id.ends_with(QUERY) || !matches!(row.delete_flag, None | Some(0)) {
                return;
            }
            if let Some(name) = [row.remark.as_deref(), row.nickname.as_deref()]
                .into_iter().flatten().map(str::trim).find(|s| !s.is_empty() && *s != id.as_str())
            {
                names.insert(id.clone(), name.to_owned());
            }
        })?;
        Ok(names)
    }

    /// Only current-account entries classified as friends by the local directory.
    pub fn directory_friends_local(&self, account: &str, request_id: &str) -> AppResult<Vec<DirectoryFriend>> {
        use crate::directory_export::{contact_category, DirectoryCategory};
        let current = self.checked_send_context(account, "filehelper")?;
        let mut friends = std::collections::BTreeMap::new();
        self.visit_bound_contacts(account, request_id, None, |row| {
            if row.anchor.username != current.0.sender_id
                && contact_category(row) == DirectoryCategory::Friend
            {
                let username = row.anchor.username.clone();
                friends.insert(username.clone(), DirectoryFriend {
                    username,
                    nickname: row.nickname.clone(),
                    remark: row.remark.clone(),
                });
            }
        })?;
        if self.checked_send_context(account, "filehelper")? != current {
            return Err(stale());
        }
        Ok(friends.into_values().collect())
    }

    /// Categorized local entries, without message history or relationship inference.
    pub fn directory_contacts_local(&self, account: &str, request_id: &str) -> AppResult<Vec<DirectoryContact>> {
        use crate::directory_export::contact_category;
        let current = self.checked_send_context(account, "filehelper")?;
        let mut contacts = std::collections::BTreeMap::new();
        self.visit_bound_contacts(account, request_id, None, |row| {
            if row.anchor.username != current.0.sender_id {
                let username = row.anchor.username.clone();
                contacts.insert(username.clone(), DirectoryContact {
                    contact: DirectoryFriend { username, nickname: row.nickname.clone(), remark: row.remark.clone() },
                    category: contact_category(row).key(),
                });
            }
        })?;
        if self.checked_send_context(account, "filehelper")? != current {
            return Err(stale());
        }
        Ok(contacts.into_values().collect())
    }

    fn visit_bound_contacts(
        &self, account: &str, request_id: &str, query: Option<&str>,
        mut visit: impl FnMut(&LiveContact),
    ) -> AppResult<()> {
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        let selected = self.selected_automatic_sources_local(account)?;
        let database = selected.databases.first().ok_or_else(|| {
            AppError::new("SOURCE_NOT_FOUND", "请先在 WeRC 绑定当前微信来源")
        })?;
        // This also rejects a discovered but not yet bound root.
        let (binding, _, source) = self.live_read_binding(account, database)?;
        let mut after: Option<LiveContactContinuation> = None;
        let mut scanned = 0;
        loop {
            lease.check()?;
            let (fingerprint, window) = self.provider.read_live_contacts_filtered(
                account, database, &source, after.as_ref(), query, &lease.cancel,
            )?;
            if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                || after.as_ref().is_some_and(|a| a.source_fingerprint != fingerprint)
                || !window.valid(after.as_ref().map(|a| &a.anchor))
            {
                return Err(AppError::new("LIVE_PROTOCOL_INVALID", "联系人分页来源或结果无效"));
            }
            scanned += window.rows.len();
            if scanned > 50_000 {
                return Err(AppError::new("DIRECTORY_EXPORT_LIMIT", "联系人查询超过5万项"));
            }
            for row in &window.rows { visit(row); }
            if !window.has_more { break; }
            after = Some(LiveContactContinuation {
                source_fingerprint: fingerprint,
                anchor: window.rows.last().ok_or_else(stale)?.anchor.clone(),
                search_query: query.map(str::to_owned),
            });
        }
        lease.check()?;
        if self.live_read_binding(account, database)?.0 != binding {
            return Err(stale());
        }
        Ok(())
    }

    /// Read the account's one local contact cache through its authenticated source set.
    pub fn live_contacts_local(
        &self,
        account: &str,
        view_id: &str,
        cursor: Option<&str>,
        request_id: &str,
    ) -> AppResult<LiveContactPage> {
        self.live_contacts_search_local(account, view_id, cursor, None, request_id)
    }
    pub fn live_contacts_search_local(
        &self,
        account: &str,
        view_id: &str,
        cursor: Option<&str>,
        search_query: Option<&str>,
        request_id: &str,
    ) -> AppResult<LiveContactPage> {
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        self.read_contacts_with_lease(account, view_id, cursor, search_query, &lease)
    }
    pub(crate) fn read_contacts_with_lease(
        &self,
        account: &str,
        view_id: &str,
        cursor: Option<&str>,
        search_query: Option<&str>,
        lease: &crate::history::QueryLease<'_>,
    ) -> AppResult<LiveContactPage> {
        let search_query = search_query.map(str::trim).filter(|q| !q.is_empty());
        if search_query.is_some_and(|q| !valid_contact_query(q)) {
            return Err(AppError::new(
                "LIVE_QUERY_INVALID",
                "联系人搜索文字最多256个UTF-8字节",
            ));
        }
        lease.check()?;
        let (database, binding, source, after) =
            self.with_live_source_parent(account, view_id, |database, binding, source, view| {
                let after = cursor
                    .map(|c| view.open_contacts(binding, view_id, c))
                    .transpose()?;
                Ok((
                    zeroize::Zeroizing::new(database.to_owned()),
                    binding.to_owned(),
                    source.to_owned(),
                    after,
                ))
            })?;
        let query = after
            .as_ref()
            .and_then(|a| a.search_query.as_deref())
            .or(search_query);
        if after
            .as_ref()
            .is_some_and(|a| search_query.is_some_and(|q| a.search_query.as_deref() != Some(q)))
        {
            return Err(stale());
        }
        if query.is_some_and(|q| !valid_contact_query(q)) {
            return Err(stale());
        }
        let (fingerprint, window) = self.provider.read_live_contacts_filtered(
            account,
            &database,
            &source,
            after.as_ref(),
            query,
            &lease.cancel,
        )?;
        lease.check()?;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || after
                .as_ref()
                .is_some_and(|a| a.source_fingerprint != fingerprint)
            || !window.valid(after.as_ref().map(|a| &a.anchor))
            || query.is_some_and(|q| window.rows.iter().any(|row| !row.matches_query(q)))
        {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "联系人分页来源或结果无效",
            ));
        }
        let plain =
            zeroize::Zeroizing::new(serde_json::to_vec(&window).map_err(|_| AppError::internal())?);
        if plain.len() > LIVE_CONTACT_MAX_BYTES {
            return Err(AppError::new(
                "LIVE_CONTACT_WINDOW_LIMIT",
                "联系人页超过读取上限",
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
                    Some(view.seal_contacts(
                        &binding,
                        view_id,
                        &LiveContactContinuation {
                            search_query: query.map(str::to_owned),
                            source_fingerprint: fingerprint,
                            anchor: window.rows.last().ok_or_else(stale)?.anchor.clone(),
                        },
                    )?)
                } else {
                    None
                };
                Ok(LiveContactPage {
                    search_query: query.map(str::to_owned),
                    rows: window.rows,
                    next,
                    expires_in_seconds,
                    mode: "local_contact_cache",
                    complete_address_book: false,
                    cross_page_snapshot: false,
                })
            },
        )
    }
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "在线来源窗口已变化或过期，请重新读取")
}
