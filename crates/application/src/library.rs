//! Local personal records. Content is encrypted; edits never reach WeChat.
use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use werc_domain::{AppError, AppResult, library::*};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
pub struct LibraryEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<werc_domain::asr::Transcript>,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
}
#[derive(Serialize)]
pub struct LibraryItem {
    pub header: DocumentHeader,
    pub entry: LibraryEntry,
}
#[derive(Serialize)]
pub struct LibraryPage {
    pub data: Vec<LibraryItem>,
    pub next_cursor: Option<i64>,
}
fn corrupt() -> AppError {
    AppError::new("DOCUMENT_CORRUPTED", "资料完整性校验失败")
}
fn kind(value: &str) -> AppResult<()> {
    if !matches!(
        value,
        "favorite" | "note" | "article" | "template" | "transcript" | "report"
    ) {
        return Err(AppError::new("INVALID_ARGUMENT", "资料分类无效"));
    }
    Ok(())
}
impl Application {
    pub(crate) fn seal_document<T: Serialize>(
        &self,
        account: &str,
        id: &str,
        kind: &str,
        revision: i64,
        data: &T,
    ) -> AppResult<ProtectedDocument> {
        self.ensure_account(account)?;
        self.seal_document_unchecked(account, id, kind, revision, data)
    }
    pub(crate) fn seal_document_unchecked<T: Serialize>(
        &self,
        account: &str,
        id: &str,
        kind: &str,
        revision: i64,
        data: &T,
    ) -> AppResult<ProtectedDocument> {
        let header = DocumentHeader {
            account_id: account.into(),
            id: id.into(),
            kind: kind.into(),
            revision,
        };
        let plain = Zeroizing::new(serde_json::to_vec(data).map_err(|_| AppError::internal())?);
        if plain.len() > MAX_DOCUMENT_BYTES {
            return Err(AppError::new("QUOTA_EXCEEDED", "单份资料超过4MiB，请拆分"));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = serde_json::to_vec(&header).map_err(|_| AppError::internal())?;
        let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| corrupt())?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &aad,
                },
            )
            .map_err(|_| corrupt())?;
        let wrapped_key = self
            .provider
            .protect_key(&format!("WeRC/library/v1/{account}/{id}"), key.as_ref())?;
        Ok(ProtectedDocument {
            header,
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }
    pub(crate) fn open_document<T: DeserializeOwned>(
        &self,
        account: &str,
        id: &str,
        kind: &str,
    ) -> AppResult<(DocumentHeader, T)> {
        self.ensure_account(account)?;
        let d = self.repository.library_get(account, id)?;
        if d.header.account_id != account
            || d.header.id != id
            || d.header.kind != kind
            || d.header.revision <= 0
            || d.nonce.len() != 12
        {
            return Err(corrupt());
        }
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&format!("WeRC/library/v1/{account}/{id}"), &d.wrapped_key)?,
        );
        let aad = serde_json::to_vec(&d.header).map_err(|_| AppError::internal())?;
        let plain = Zeroizing::new(
            Aes256Gcm::new_from_slice(&key)
                .map_err(|_| corrupt())?
                .decrypt(
                    Nonce::from_slice(&d.nonce),
                    Payload {
                        msg: &d.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| corrupt())?,
        );
        let data = serde_json::from_slice(&plain).map_err(|_| corrupt())?;
        Ok((d.header, data))
    }
    pub fn library_save_local(
        &self,
        account: &str,
        id: Option<&str>,
        category: &str,
        expected_revision: i64,
        entry: LibraryEntry,
    ) -> AppResult<LibraryItem> {
        kind(category)?;
        validate_entry(category, &entry)?;
        if expected_revision < 0 || expected_revision == i64::MAX {
            return Err(AppError::new("INVALID_ARGUMENT", "资料版本无效"));
        }
        let id = if let Some(id) = id {
            let (h, _): (_, LibraryEntry) = self.open_document(account, id, category)?;
            if h.revision != expected_revision {
                return Err(AppError::new("DOCUMENT_CONFLICT", "资料已更新，请重新加载"));
            }
            id.to_owned()
        } else {
            if expected_revision != 0 {
                return Err(AppError::new("INVALID_ARGUMENT", "新资料版本应为0"));
            }
            uuid::Uuid::new_v4().to_string()
        };
        let document = self.seal_document(account, &id, category, expected_revision + 1, &entry)?;
        let header = document.header.clone();
        self.repository.library_save(&[document], None)?;
        Ok(LibraryItem { header, entry })
    }
    pub fn library_page_local(
        &self,
        account: &str,
        category: &str,
        before: Option<i64>,
    ) -> AppResult<LibraryPage> {
        kind(category)?;
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, category, before, 20)?;
        let data = page
            .data
            .into_iter()
            .map(|h| {
                self.open_document(account, &h.id, category)
                    .map(|(header, entry)| LibraryItem { header, entry })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(LibraryPage {
            data,
            next_cursor: page.next_cursor,
        })
    }
    pub fn library_item_local(
        &self,
        account: &str,
        id: &str,
        category: &str,
    ) -> AppResult<LibraryItem> {
        kind(category)?;
        let (header, entry) = self.open_document(account, id, category)?;
        Ok(LibraryItem { header, entry })
    }
    pub fn library_delete_local(
        &self,
        account: &str,
        id: &str,
        category: &str,
        revision: i64,
    ) -> AppResult<()> {
        kind(category)?;
        let (h, _): (_, LibraryEntry) = self.open_document(account, id, category)?;
        if revision != h.revision {
            return Err(AppError::new("DOCUMENT_CONFLICT", "资料已更新，请重新加载"));
        }
        self.repository.library_delete(account, id, revision)
    }
}

pub(crate) fn validate_entry(category: &str, entry: &LibraryEntry) -> AppResult<()> {
    if let Some(t) = &entry.transcript {
        if category != "transcript"
            || t.text != entry.text
            || t.segments.len() > 100000
            || t.segments
                .iter()
                .any(|s| s.from_ms > s.until_ms || s.until_ms > 86400000 || s.text.len() > 16384)
            || t.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                != t.text
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "转写正文与字幕分段不一致",
            ));
        }
    }
    if entry.title.trim().is_empty()
        || entry.title.len() > 512
        || entry.text.len() > 2 * 1024 * 1024
        || entry.tags.len() > 32
        || entry.tags.iter().any(|t| t.len() > 128)
        || entry.source.as_ref().is_some_and(|s| s.len() > 4096)
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "标题、正文或标签超出范围",
        ));
    }
    Ok(())
}
