//! Fetch only descriptors selected from this account's authenticated SNS cache.
use reqwest::{Url, blocking::Client};
use sha2::Digest;
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use werc_domain::{AppError, AppResult, moments::*};
use zeroize::Zeroizing;
fn error(code: &str) -> AppError {
    AppError::new(code, "朋友圈媒体读取失败，已有内容不受影响")
}
fn check(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(error("QUERY_CANCELED"))
    } else {
        Ok(())
    }
}
fn checked_url(text: &str) -> AppResult<Url> {
    let mut url = Url::parse(text).map_err(|_| error("MOMENT_MEDIA_URL_INVALID"))?;
    let host = url
        .host_str()
        .ok_or_else(|| error("MOMENT_MEDIA_URL_INVALID"))?;
    if ![".qpic.cn", ".video.qq.com", ".tc.qq.com"]
        .iter()
        .any(|s| host.ends_with(s))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "http" | "https")
        || url.port().is_some_and(|p| p != 80 && p != 443)
    {
        return Err(error("MOMENT_MEDIA_HOST_UNSUPPORTED"));
    }
    url.set_scheme("https")
        .map_err(|_| error("MOMENT_MEDIA_URL_INVALID"))?;
    url.set_port(None)
        .map_err(|_| error("MOMENT_MEDIA_URL_INVALID"))?;
    Ok(url)
}
fn resource_url(d: &MomentMediaDescriptor, original: bool) -> AppResult<Url> {
    let mut url = checked_url(&d.url)?;
    if original && !d.video {
        if let Some((prefix, last)) = url.path().rsplit_once('/') {
            if matches!(last, "150" | "200" | "480") {
                let path = format!("{prefix}/0");
                url.set_path(&path);
            }
        }
    }
    if !d.token.is_empty() {
        // SNS tokens are already URL-escaped; query_pairs_mut would encode '%' a
        // second time. Keep only a token value, never caller-controlled parameters.
        if d.token.len() > 4096
            || !d
                .token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"%+/_=-".contains(&b))
        {
            return Err(error("MOMENT_MEDIA_TOKEN_INVALID"));
        }
        let existing = url
            .query()
            .unwrap_or("")
            .split('&')
            .filter(|p| !p.is_empty() && !p.starts_with("token=") && !p.starts_with("idx="))
            .collect::<Vec<_>>()
            .join("&");
        let query = format!(
            "token={}&idx=1{}{}",
            d.token,
            if existing.is_empty() { "" } else { "&" },
            existing
        );
        url.set_query(Some(&query));
    }
    Ok(url)
}
pub(crate) fn download(
    d: &MomentMediaDescriptor,
    original: bool,
    cancel: &AtomicBool,
) -> AppResult<MomentMediaBytes> {
    check(cancel)?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| error("MOMENT_MEDIA_NETWORK"))?;
    let mut url = resource_url(d, original)?;
    let mut response = None;
    for hop in 0..=3 {
        check(cancel)?;
        let r = client
            .get(url.clone())
            .header("User-Agent", "MicroMessenger Client")
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity")
            .send()
            .map_err(|_| error("MOMENT_MEDIA_NETWORK"))?;
        if r.status().is_redirection() {
            if hop == 3 {
                return Err(error("MOMENT_MEDIA_REDIRECT_LIMIT"));
            }
            let location = r
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| error("MOMENT_MEDIA_REDIRECT_INVALID"))?;
            let next = url
                .join(location)
                .map_err(|_| error("MOMENT_MEDIA_REDIRECT_INVALID"))?;
            url = checked_url(next.as_str())?;
            continue;
        }
        if !r.status().is_success() || r.status().as_u16() != 200 {
            return Err(error("MOMENT_MEDIA_UNAVAILABLE"));
        }
        response = Some(r);
        break;
    }
    let mut r = response.ok_or_else(|| error("MOMENT_MEDIA_NETWORK"))?;
    let limit = werc_domain::video::MAX_VIDEO_BYTES;
    if r.content_length().is_some_and(|n| n > limit as u64) {
        return Err(error("MOMENT_MEDIA_SIZE_LIMIT"));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    let mut part = Zeroizing::new([0u8; 65536]);
    loop {
        check(cancel)?;
        let n = r
            .read(part.as_mut())
            .map_err(|_| error("MOMENT_MEDIA_NETWORK"))?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > limit {
            return Err(error("MOMENT_MEDIA_SIZE_LIMIT"));
        }
        bytes.extend_from_slice(&part[..n]);
    }
    check(cancel)?;
    let raw_md5 = hex::encode(md5::Md5::digest(&*bytes));
    // An already readable payload needs no decryption (some CDN responses are plain).
    let readable = if d.video {
        bytes.get(4..8) == Some(b"ftyp")
    } else {
        werc_domain::image::canonical_received_png(&bytes).is_ok()
    };
    if !readable {
        let seed = d.seed.ok_or_else(|| error("MOMENT_MEDIA_KEY_REQUIRED"))?;
        let length = if d.video {
            bytes.len().min(131072)
        } else {
            bytes.len()
        };
        isaac::Isaac64::new(seed).xor(&mut bytes[..length]);
    }
    let decoded_md5 = hex::encode(md5::Md5::digest(&*bytes));
    let source_md5_matches = d
        .md5
        .as_ref()
        .map(|m| m.eq_ignore_ascii_case(&raw_md5) || m.eq_ignore_ascii_case(&decoded_md5));
    let (bytes, mime, extension) = if d.video {
        (bytes, "video/mp4", "mp4")
    } else {
        (
            werc_domain::image::canonical_received_png(&bytes)?.1,
            "image/png",
            "png",
        )
    };
    check(cancel)?;
    Ok(MomentMediaBytes {
        moment: d.moment.clone(),
        bytes,
        mime,
        extension,
        source_md5_matches,
    })
}
