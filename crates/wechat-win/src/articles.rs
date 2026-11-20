use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use werc_domain::{AppError, AppResult, articles::*};
fn check(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(AppError::new("QUERY_CANCELED", "文章读取已取消"))
    } else {
        Ok(())
    }
}
pub fn fetch_avatar(value:&str,cancel:&AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{
    let valid=|v:&str| werc_domain::live::contact_avatar_url(v).ok_or_else(||AppError::new("CONTACT_AVATAR_UNAVAILABLE","头像地址未提供或不受支持"));
    let mut url=valid(value)?;
    let client=reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(6)).timeout(Duration::from_secs(12)).build().map_err(|_|AppError::internal())?;
    for hop in 0..=3 {
        check(cancel)?;
        let mut response=client.get(&url).header("Accept-Encoding","identity").send().map_err(|_|AppError::new("CONTACT_AVATAR_NETWORK","头像连接失败或超时"))?;
        if response.status().is_redirection(){
            if hop==3 {break;}
            let location=response.headers().get("location").and_then(|s|s.to_str().ok()).ok_or_else(||AppError::new("CONTACT_AVATAR_UNAVAILABLE","头像跳转地址无效"))?;
            url=valid(response.url().join(location).map_err(|_|AppError::internal())?.as_str())?;continue;
        }
        if !response.status().is_success(){break;}
        let mut bytes=zeroize::Zeroizing::new(Vec::new());let mut block=[0u8;8192];
        loop{check(cancel)?;let n=response.read(&mut block).map_err(|_|AppError::new("CONTACT_AVATAR_NETWORK","头像读取失败"))?;if n==0{break;}if bytes.len()+n>1024*1024{return Err(AppError::new("CONTACT_AVATAR_TOO_LARGE","头像超过1MiB"));}bytes.extend_from_slice(&block[..n]);}
        let (info,png)=werc_domain::image::canonical_upload_png(&bytes)?;
        if info.width>1024||info.height>1024||png.len()>1024*1024{return Err(AppError::new("CONTACT_AVATAR_TOO_LARGE","头像尺寸或大小超过上限"));}
        check(cancel)?;return Ok(png);
    }
    Err(AppError::new("CONTACT_AVATAR_UNAVAILABLE","头像已失效或暂不可用"))
}
pub fn fetch_image(value:&str,cancel:&AtomicBool)->AppResult<ArticleImageBytes>{
    let mut url=image_url(value)?;
    let client=reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20)).build().map_err(|_|AppError::internal())?;
    for hop in 0..=3 {
        check(cancel)?;
        let mut response=client.get(&url).header("Accept-Encoding","identity").send().map_err(|_|AppError::new("ARTICLE_IMAGE_NETWORK","文章图片连接失败或超时"))?;
        if response.status().is_redirection(){
            if hop==3{return Err(AppError::new("ARTICLE_IMAGE_REDIRECT","文章图片跳转过多"));}
            let location=response.headers().get("location").and_then(|s|s.to_str().ok()).ok_or_else(||AppError::new("ARTICLE_IMAGE_REDIRECT","图片跳转地址无效"))?;
            url=image_url(response.url().join(location).map_err(|_|AppError::internal())?.as_str())?;continue;
        }
        if !response.status().is_success(){return Err(AppError::new("ARTICLE_IMAGE_UNAVAILABLE","微信图片服务未返回图片"));}
        let mut bytes=zeroize::Zeroizing::new(Vec::new());let mut block=[0u8;16384];
        loop{check(cancel)?;let n=response.read(&mut block).map_err(|_|AppError::new("ARTICLE_IMAGE_NETWORK","文章图片读取失败"))?;if n==0{break;}if bytes.len()+n>werc_domain::image::MAX_IMAGE_BYTES{return Err(AppError::new("ARTICLE_IMAGE_TOO_LARGE","文章图片超过8MiB"));}bytes.extend_from_slice(&block[..n]);}
        check(cancel)?;
        if bytes.starts_with(b"RIFF")&&bytes.get(8..12)==Some(b"WEBP"){
            validate_webp(&bytes,cancel)?;return Ok(ArticleImageBytes{bytes,mime:"image/webp".into(),extension:"webp".into()});
        }
        if bytes.starts_with(b"GIF8"){
            werc_domain::emoticon::inspect_article_gif(&bytes,cancel)?;
            return Ok(ArticleImageBytes{bytes,mime:"image/gif".into(),extension:"gif".into()});
        }
        let (_,bytes)=werc_domain::image::canonical_upload_png(&bytes)?;
        return Ok(ArticleImageBytes{bytes,mime:"image/png".into(),extension:"png".into()});
    }
    Err(AppError::new("ARTICLE_IMAGE_NETWORK","文章图片读取未完成"))
}
pub fn fetch_public(value: &str, cancel: &AtomicBool) -> AppResult<Article> {
    let mut url = canonical_url(value)?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::new("ARTICLE_NETWORK", "文章网络组件不可用"))?;
    for hop in 0..=3 {
        check(cancel)?;
        let mut response = client
            .get(&url)
            .header("Accept-Encoding", "identity")
            .header("User-Agent", "Mozilla/5.0 WeRC Article Reader")
            .send()
            .map_err(|_| AppError::new("ARTICLE_NETWORK", "文章连接失败或超时"))?;
        if response.status().is_redirection() {
            if hop == 3 {
                return Err(AppError::new("ARTICLE_REDIRECT_LIMIT", "文章跳转次数过多"));
            }
            let next = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| AppError::new("ARTICLE_REDIRECT_INVALID", "文章跳转地址无效"))?;
            let next = response
                .url()
                .join(next)
                .map_err(|_| AppError::new("ARTICLE_REDIRECT_INVALID", "文章跳转地址无效"))?;
            url = canonical_url(next.as_str()).map_err(|_| {
                AppError::new(
                    "ARTICLE_LOGIN_REQUIRED",
                    "文章跳转至验证或登录页，请在微信内确认访问状态",
                )
            })?;
            continue;
        }
        if !response.status().is_success() {
            return Err(AppError::new(
                "ARTICLE_UNAVAILABLE",
                &format!("文章服务返回 {}", response.status().as_u16()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_ARTICLE_HTML as u64)
        {
            return Err(AppError::new("ARTICLE_TOO_LARGE", "文章超过读取上限"));
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        let mut buffer = [0u8; 16384];
        loop {
            check(cancel)?;
            let n = response
                .read(&mut buffer)
                .map_err(|_| AppError::new("ARTICLE_NETWORK", "文章正文读取失败"))?;
            if n == 0 {
                break;
            }
            if bytes.len() + n > MAX_ARTICLE_HTML {
                return Err(AppError::new("ARTICLE_TOO_LARGE", "文章超过读取上限"));
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
        check(cancel)?;
        let html = std::str::from_utf8(&bytes)
            .map_err(|_| AppError::new("ARTICLE_ENCODING_UNSUPPORTED", "文章编码不是UTF-8"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AppError::internal())?
            .as_secs() as i64;
        return parse_article(&url, html, "public_web", now);
    }
    Err(AppError::new("ARTICLE_NETWORK", "文章读取未完成"))
}
