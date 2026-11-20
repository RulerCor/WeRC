//! Complete local-directory exports through the same authenticated paging path as browsing.
pub(crate) mod xlsx;
use crate::Application;
use serde::{Deserialize, Serialize};
use std::io::Write;
use werc_domain::{AppError, AppResult, group::LiveGroupMember, live::LiveContact};
use zeroize::Zeroizing;

const MAX_ROWS: usize = 50_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryCategory {
    Group,
    Service,
    Deleted,
    Other,
    Friend,
    Enterprise,
    EnterpriseGroup,
    Subscription,
    OfficialService,
    Blocked,
    Former,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectorySelection {
    Contacts {
        search_query: Option<String>,
        #[serde(default)]
        categories: Vec<DirectoryCategory>,
    },
    Group {
        username: String,
    },
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryFormat {
    Zip,
    Xlsx,
    Json,
    Csv,
    Txt,
}
impl DirectoryFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Xlsx => "xlsx",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Txt => "txt",
        }
    }
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Zip => "application/zip",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Json => "application/json",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Txt => "text/plain; charset=utf-8",
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "record_type", content = "record", rename_all = "snake_case")]
enum DirectoryRow {
    Contact(LiveContact),
    GroupMember(LiveGroupMember),
}
#[derive(Serialize)]
pub struct DirectoryExportSummary {
    pub rows: usize,
    pub pages: usize,
    pub avatars_written: usize,
    pub complete_local_selection: bool,
    pub complete_server_directory: bool,
    pub cross_page_snapshot: bool,
    pub bytes: usize,
}
pub struct DirectoryExport {
    pub bytes: Zeroizing<Vec<u8>>,
    pub summary: DirectoryExportSummary,
}
pub fn contact_category(row: &LiveContact) -> DirectoryCategory {
    if row.delete_flag.is_some_and(|n| n != 0) {
        DirectoryCategory::Deleted
    } else if row.anchor.username.ends_with("@im.chatroom") {
        DirectoryCategory::EnterpriseGroup
    } else if row.anchor.username.ends_with("@chatroom") {
        DirectoryCategory::Group
    } else if row.anchor.username.starts_with("gh_") {
        match row.business_type {
            Some(0) => DirectoryCategory::Subscription,
            Some(1) => DirectoryCategory::OfficialService,
            _ => DirectoryCategory::Service,
        }
    } else if row.contact_flag.is_some_and(|n| n >= 0 && n & 8 != 0) {
        DirectoryCategory::Blocked
    } else if row.anchor.username.ends_with("@openim") {
        DirectoryCategory::Enterprise
    } else if row.local_type == Some(1)
        && !matches!(
            row.anchor.username.as_str(),
            "filehelper"
                | "fmessage"
                | "medianote"
                | "floatbottle"
                | "newsapp"
                | "weixin"
                | "notifymessage"
                | "brandsessionholder"
                | "qqmail"
                | "weibo"
                | "feedsapp"
                | "voipapp"
                | "lbsapp"
                | "shakeapp"
                | "readerapp"
                | "blogapp"
                | "facebookapp"
                | "masssendapp"
                | "meishiapp"
                | "linkedinplugin"
                | "qqsync"
        )
    {
        DirectoryCategory::Friend
    } else if row.local_type == Some(3)
        && row.contact_flag.is_some_and(|n| n != 4)
        && [&row.alias, &row.remark]
            .iter()
            .any(|s| s.as_ref().is_some_and(|s| !s.is_empty()))
    {
        DirectoryCategory::Former
    } else {
        DirectoryCategory::Other
    }
}
impl DirectoryCategory {
    pub fn key(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Service => "service",
            Self::Deleted => "deleted",
            Self::Other => "other",
            Self::Friend => "friend",
            Self::Enterprise => "enterprise",
            Self::EnterpriseGroup => "enterprise_group",
            Self::Subscription => "subscription",
            Self::OfficialService => "official_service",
            Self::Blocked => "blocked",
            Self::Former => "former",
        }
    }
}
fn category_matches(row: &LiveContact, category: DirectoryCategory) -> bool {
    contact_category(row) == category
        || (category == DirectoryCategory::Service
            && row.anchor.username.starts_with("gh_")
            && row.delete_flag.is_none_or(|n| n == 0))
}
fn limit() -> AppError {
    AppError::new(
        "DIRECTORY_EXPORT_LIMIT",
        "本次导出超过5万条或32MiB，请缩小搜索范围；未写入文件",
    )
}
impl Application {
    pub fn export_live_directory_local<T>(
        &self,
        account: &str,
        view_id: &str,
        selection: &DirectorySelection,
        format: DirectoryFormat,
        request_id: &str,
        finish: impl FnOnce(DirectoryExport) -> AppResult<T>,
    ) -> AppResult<T> {
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let lease = self.browse_lease(request_id)?;
        lease.check()?;
        if let DirectorySelection::Contacts {
            search_query,
            categories,
        } = selection
        {
            if categories.len() > 11
                || search_query.as_deref().is_some_and(|q| {
                    !q.trim().is_empty() && !werc_domain::live::valid_contact_query(q.trim())
                })
            {
                return Err(AppError::new("INVALID_ARGUMENT", "联系人导出条件无效"));
            }
        }
        let mut rows = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        let mut retained = 0;
        loop {
            lease.check()?;
            let (batch, next) = match selection {
                DirectorySelection::Contacts {
                    search_query,
                    categories,
                } => {
                    let page = self.read_contacts_with_lease(
                        account,
                        view_id,
                        cursor.as_deref(),
                        search_query.as_deref(),
                        &lease,
                    )?;
                    let rows = page
                        .rows
                        .into_iter()
                        .filter(|r| {
                            categories.is_empty()
                                || categories.iter().any(|c| category_matches(r, *c))
                        })
                        .map(DirectoryRow::Contact)
                        .collect::<Vec<_>>();
                    (rows, page.next)
                }
                DirectorySelection::Group { username } => {
                    let page = self.read_groups_with_lease(
                        account,
                        view_id,
                        username,
                        cursor.as_deref(),
                        &lease,
                    )?;
                    (
                        page.rows
                            .into_iter()
                            .map(DirectoryRow::GroupMember)
                            .collect(),
                        page.next,
                    )
                }
            };
            pages += 1;
            for row in batch {
                retained +=
                    Zeroizing::new(serde_json::to_vec(&row).map_err(|_| AppError::internal())?)
                        .len();
                if rows.len() >= MAX_ROWS || retained > MAX_BYTES {
                    return Err(limit());
                }
                rows.push(row);
            }
            let Some(next) = next else { break };
            if cursor.as_ref() == Some(&next) || pages > 1000 {
                return Err(AppError::new(
                    "LIVE_PROTOCOL_INVALID",
                    "来源分页没有完成，未写入导出文件",
                ));
            }
            cursor = Some(next);
        }
        let (bytes, avatars_written) = if matches!(format, DirectoryFormat::Zip) {
            self.directory_bundle(account, selection, &rows, &lease)?
        } else {
            (render(account, selection, &rows, format)?, 0)
        };
        self.with_live_source_parent(account, view_id, |_, _, _, _| {
            lease.check()?;
            Ok(())
        })?;
        let summary = DirectoryExportSummary {
            rows: rows.len(),
            pages,
            avatars_written,
            complete_local_selection: true,
            complete_server_directory: false,
            cross_page_snapshot: false,
            bytes: bytes.len(),
        };
        finish(DirectoryExport { bytes, summary })
    }

    fn directory_bundle(
        &self,
        account: &str,
        selection: &DirectorySelection,
        rows: &[DirectoryRow],
        lease: &crate::history::QueryLease<'_>,
    ) -> AppResult<(Zeroizing<Vec<u8>>, usize)> {
        use sha2::{Digest, Sha256};
        use std::io::Cursor;
        use zip::{ZipWriter, write::SimpleFileOptions};
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        let mut bytes = 0usize;
        let mut written = 0usize;
        let mut manifest = Vec::new();
        let mut add = |name: &str, data: &[u8]| -> AppResult<()> {
            bytes = bytes.checked_add(data.len()).ok_or_else(limit)?;
            if bytes > MAX_BYTES {
                return Err(limit());
            }
            out.start_file(name, SimpleFileOptions::default())
                .map_err(|_| AppError::internal())?;
            out.write_all(data).map_err(|_| AppError::internal())
        };
        for (name, format) in [
            ("contacts.json", DirectoryFormat::Json),
            ("contacts.csv", DirectoryFormat::Csv),
        ] {
            lease.check()?;
            add(name, &render(account, selection, rows, format)?)?;
        }
        for row in rows {
            lease.check()?;
            let DirectoryRow::Contact(contact) = row else {
                continue;
            };
            let Some(url) = &contact.avatar_url else {
                manifest.push(
                    serde_json::json!({"username":contact.anchor.username,"status":"unavailable"}),
                );
                continue;
            };
            if let Some(avatar) =
                self.cached_contact_avatar(account, &contact.anchor.username, url)?
            {
                let image = avatar.bytes()?;
                let path = format!(
                    "avatars/{}.png",
                    hex::encode(Sha256::digest(contact.anchor.username.as_bytes()))
                );
                add(&path, &image)?;
                written += 1;
                manifest.push(serde_json::json!({"username":contact.anchor.username,"status":"saved","path":path,"sha256":avatar.sha256,"bytes":image.len()}));
            } else {
                manifest.push(
                    serde_json::json!({"username":contact.anchor.username,"status":"not_cached"}),
                );
            }
        }
        add(
            "avatars.json",
            &serde_json::to_vec_pretty(&manifest).map_err(|_| AppError::internal())?,
        )?;
        add("README.txt","WeRC 本地通讯录\ncontacts.json / contacts.csv 保存所选来源的全部续页记录。\navatars/仅包含此前主动读取并加密缓存的头像，不会在导出时联网。\navatars.json逐项记录已保存、未缓存或不可用；所有引用均为相对路径。\n本地缓存不是服务器实时好友关系；跨页不是同一快照。\n".as_bytes())?;
        drop(add);
        lease.check()?;
        let data = out.finish().map_err(|_| AppError::internal())?.into_inner();
        if data.len() > MAX_BYTES {
            return Err(limit());
        }
        Ok((Zeroizing::new(data), written))
    }
}

pub(crate) struct Output(pub(crate) Zeroizing<Vec<u8>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(std::io::Error::other("directory export limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn csv(out: &mut Output, text: &str, identifier: bool) -> std::io::Result<()> {
    out.write_all(b"\"")?;
    // Preserve identifiers as text and prevent spreadsheet formula evaluation.
    if identifier && !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
        || text
            .trim_start_matches([' ', '\u{feff}', '\n', '\r', '\t'])
            .starts_with(['=', '+', '-', '@'])
        || text.starts_with(['\t', '\r'])
    {
        out.write_all(b"'")?;
    }
    for part in text.split_inclusive('"') {
        out.write_all(part.as_bytes())?;
        if part.ends_with('"') {
            out.write_all(b"\"")?;
        }
    }
    out.write_all(b"\"")
}
fn fields(row: &DirectoryRow) -> [String; 14] {
    match row {
        DirectoryRow::Contact(r) => [
            "contact".into(),
            r.anchor.row_id.clone(),
            r.anchor.username.clone(),
            r.nickname.clone().unwrap_or_default(),
            r.remark.clone().unwrap_or_default(),
            r.alias.clone().unwrap_or_default(),
            r.local_type.map(|n| n.to_string()).unwrap_or_default(),
            r.delete_flag.map(|n| n.to_string()).unwrap_or_default(),
            r.verify_flag.map(|n| n.to_string()).unwrap_or_default(),
            String::new(),
            String::new(),
            r.contact_flag.map(|n| n.to_string()).unwrap_or_default(),
            r.business_type.map(|n| n.to_string()).unwrap_or_default(),
            contact_category(r).key().into(),
        ],
        DirectoryRow::GroupMember(r) => [
            "group_member".into(),
            r.anchor.row_id.clone(),
            r.anchor.username.clone().unwrap_or_default(),
            r.nickname.clone().unwrap_or_default(),
            r.remark.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            r.anchor.member_id.clone().unwrap_or_default(),
            r.group_nickname.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            String::new(),
        ],
    }
}
fn render(
    account: &str,
    selection: &DirectorySelection,
    rows: &[DirectoryRow],
    format: DirectoryFormat,
) -> AppResult<Zeroizing<Vec<u8>>> {
    if matches!(format, DirectoryFormat::Xlsx) {
        return xlsx::render(rows);
    }
    let mut out = Output(Zeroizing::new(Vec::new()));
    let result = (|| -> std::io::Result<()> {
        match format {
            DirectoryFormat::Xlsx | DirectoryFormat::Zip => unreachable!("handled above"),
            DirectoryFormat::Json => {
                #[derive(Serialize)]
                struct Document<'a> {
                    schema_version: u32,
                    account_id: &'a str,
                    selection: &'a DirectorySelection,
                    data_mode: &'static str,
                    cross_page_snapshot: bool,
                    complete_server_directory: bool,
                    avatars_written: usize,
                    rows: &'a [DirectoryRow],
                }
                serde_json::to_writer_pretty(
                    &mut out,
                    &Document {
                        schema_version: 1,
                        account_id: account,
                        selection,
                        data_mode: "local_directory_cache",
                        cross_page_snapshot: false,
                        complete_server_directory: false,
                        avatars_written: 0,
                        rows,
                    },
                )
                .map_err(std::io::Error::other)?;
            }
            DirectoryFormat::Csv | DirectoryFormat::Txt => {
                out.write_all(b"\xef\xbb\xbf")?;
                let header = [
                    "record_type",
                    "source_row_id",
                    "username",
                    "nickname",
                    "remark",
                    "alias",
                    "local_type",
                    "delete_flag",
                    "verify_flag",
                    "member_id",
                    "group_nickname",
                    "contact_flag",
                    "business_type",
                    "local_category",
                ]
                .map(str::to_owned);
                for values in std::iter::once(header).chain(rows.iter().map(fields)) {
                    let values = Zeroizing::new(Vec::from(values));
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            out.write_all(if matches!(format, DirectoryFormat::Csv) {
                                b","
                            } else {
                                b"\t"
                            })?;
                        }
                        if matches!(format, DirectoryFormat::Csv) {
                            csv(&mut out, value, matches!(i, 1 | 2 | 9))?;
                        } else {
                            let text = Zeroizing::new(
                                value
                                    .replace('\\', "\\\\")
                                    .replace('\t', "\\t")
                                    .replace('\n', "\\n")
                                    .replace('\r', "\\r"),
                            );
                            out.write_all(text.as_bytes())?;
                        }
                    }
                    out.write_all(b"\r\n")?;
                }
            }
        }
        Ok(())
    })();
    result.map_err(|_| limit())?;
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_categories_preserve_unknown_values_and_legacy_official_selection() {
        let mut r: LiveContact =
            serde_json::from_value(serde_json::json!({"anchor":{"row_id":"1","username":"owned"}}))
                .unwrap();
        assert_eq!(contact_category(&r).key(), "other");
        r.local_type = Some(1);
        assert_eq!(contact_category(&r).key(), "friend");
        r.contact_flag = Some((1i64 << 40) | 8);
        assert_eq!(contact_category(&r).key(), "blocked");
        r.anchor.username = "owned@im.chatroom".into();
        assert_eq!(contact_category(&r).key(), "enterprise_group");
        r.anchor.username = "gh_owned".into();
        r.business_type = Some(0);
        assert_eq!(contact_category(&r).key(), "subscription");
        assert!(category_matches(&r, DirectoryCategory::Service));
        assert!(!category_matches(&r, DirectoryCategory::OfficialService));
        r.business_type = Some(1);
        assert_eq!(contact_category(&r).key(), "official_service");
        r.business_type = Some(99);
        assert_eq!(contact_category(&r).key(), "service");
        r.delete_flag = Some(1);
        assert_eq!(contact_category(&r).key(), "deleted");
        assert!(!category_matches(&r, DirectoryCategory::Service));
        r.anchor.username = "filehelper".into();
        r.delete_flag = None;
        r.contact_flag = None;
        assert_eq!(contact_category(&r).key(), "other");
        r.anchor.username = "owned".into();
        r.local_type = Some(3);
        r.remark = Some("历史".into());
        assert_eq!(contact_category(&r).key(), "other");
        r.contact_flag = Some(0);
        assert_eq!(contact_category(&r).key(), "former");
        r.contact_flag = Some(4);
        assert_eq!(contact_category(&r).key(), "other");
    }
    #[test]
    fn directory_csv_escapes_cells_and_protects_large_ids_without_changing_json() {
        let record = LiveContact {
            anchor: werc_domain::live::LiveContactAnchor {
                row_id: "9007199254740993".into(),
                username: "owned".into(),
            },
            nickname: Some("=HYPERLINK(\"x\")".into()),
            remark: Some("line\ncomma,quote\"".into()),
            alias: None,
            local_type: None,
            delete_flag: None,
            verify_flag: None,
            contact_flag: None,
            business_type: None,
            avatar_url: None,
            description: None,
        };
        let selection = DirectorySelection::Contacts {
            search_query: None,
            categories: vec![],
        };
        let rows = [DirectoryRow::Contact(record)];
        let output = render("owned", &selection, &rows, DirectoryFormat::Csv).unwrap();
        let text = std::str::from_utf8(&output).unwrap();
        assert!(text.starts_with('\u{feff}'));
        assert!(text.contains("\"'9007199254740993\""));
        assert!(text.contains("\"'=HYPERLINK(\"\"x\"\")\""));
        assert!(text.contains("\"line\ncomma,quote\"\"\""));
        let workbook = render("owned", &selection, &rows, DirectoryFormat::Xlsx).unwrap();
        assert!(workbook.starts_with(b"PK"));
        if let Some(path) = std::env::var_os("WERC_DIRECTORY_XLSX_FIXTURE") {
            std::fs::write(path, &*workbook).unwrap();
        }
        let output = render("owned", &selection, &rows, DirectoryFormat::Json).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["rows"][0]["record"]["nickname"], "=HYPERLINK(\"x\")");
        assert_eq!(
            json["rows"][0]["record"]["anchor"]["row_id"],
            "9007199254740993"
        );
        let output = render("owned", &selection, &rows, DirectoryFormat::Txt).unwrap();
        assert!(
            std::str::from_utf8(&output)
                .unwrap()
                .contains("line\\ncomma,quote\"")
        );
    }
}
