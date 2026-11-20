//! Explicit avatar loading, bound to a contact from the authenticated source.
use crate::Application;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, live::*};
use zeroize::Zeroizing;
#[derive(Serialize, Deserialize)]
pub struct ContactAvatar {
    pub username: String,
    pub url: String,
    pub sha256: String,
    pub png_hex: String,
}
fn key(username: &str, url: &str) -> String {
    format!(
        "avatar-{}",
        hex::encode(Sha256::digest(format!("{username}\0{url}")))
    )
}
fn corrupt() -> AppError {
    AppError::new("DOCUMENT_CORRUPTED", "头像副本完整性校验失败")
}
impl ContactAvatar {
    pub fn bytes(&self) -> AppResult<Zeroizing<Vec<u8>>> {
        if self.png_hex.len() > 2 * 1024 * 1024
            || contact_avatar_url(&self.url).as_deref() != Some(self.url.as_str())
        {
            return Err(corrupt());
        }
        let bytes = Zeroizing::new(hex::decode(&self.png_hex).map_err(|_| corrupt())?);
        if hex::encode(Sha256::digest(&*bytes)) != self.sha256 {
            return Err(corrupt());
        }
        let (info, _) = werc_domain::image::canonical_png(&bytes)?;
        if info.width > 1024 || info.height > 1024 {
            return Err(corrupt());
        }
        Ok(bytes)
    }
}
impl Application {
    pub(crate) fn cached_contact_avatar(
        &self,
        account: &str,
        username: &str,
        url: &str,
    ) -> AppResult<Option<ContactAvatar>> {
        self.ensure_account(account)?;
        match self.open_document::<ContactAvatar>(account, &key(username, url), "contact_avatar") {
            Ok((_, a)) => {
                if a.username != username || a.url != url {
                    return Err(corrupt());
                }
                a.bytes()?;
                return Ok(Some(a));
            }
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        // A restored backup assigns fresh IDs; its original public identity remains valid.
        let mut cursor = None;
        loop {
            let page = self
                .repository
                .library_list(account, "contact_avatar", cursor, 100)?;
            for h in page.data {
                let (_, a) =
                    self.open_document::<ContactAvatar>(account, &h.id, "contact_avatar")?;
                if a.username == username && a.url == url {
                    a.bytes()?;
                    return Ok(Some(a));
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(None);
            }
        }
    }
    pub fn contact_avatar_local(
        &self,
        account: &str,
        view: &str,
        username: &str,
        request: &str,
    ) -> AppResult<ContactAvatar> {
        if !valid_live_username(username) {
            return Err(AppError::new("INVALID_ARGUMENT", "联系人账号无效"));
        }
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let lease = self.browse_lease(request)?;
        lease.check()?;
        let mut cursor = None;
        let url = loop {
            let page = self.read_contacts_with_lease(
                account,
                view,
                cursor.as_deref(),
                Some(username),
                &lease,
            )?;
            if let Some(row) = page.rows.iter().find(|r| r.anchor.username == username) {
                break row.avatar_url.clone().ok_or_else(|| {
                    AppError::new("CONTACT_AVATAR_UNAVAILABLE", "本地联系人未保存头像地址")
                })?;
            }
            cursor = page.next;
            if cursor.is_none() {
                return Err(AppError::new("NOT_FOUND", "当前来源未找到此联系人"));
            }
        };
        if let Some(a) = self.cached_contact_avatar(account, username, &url)? {
            return Ok(a);
        }
        let bytes = self
            .provider
            .fetch_contact_avatar(account, &url, &lease.cancel)?;
        lease.check()?;
        let avatar = ContactAvatar {
            username: username.into(),
            url,
            sha256: hex::encode(Sha256::digest(&*bytes)),
            png_hex: hex::encode(&*bytes),
        };
        avatar.bytes()?;
        self.with_live_source_parent(account, view, |_, _, _, _| {
            lease.check()?;
            Ok(())
        })?;
        let doc = self.seal_document(
            account,
            &key(username, &avatar.url),
            "contact_avatar",
            1,
            &avatar,
        )?;
        self.repository.library_save(&[doc], None)?;
        Ok(avatar)
    }
}
