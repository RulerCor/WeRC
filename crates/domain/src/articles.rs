//! Article projection. No scripts, cookies or authenticated URL parameters survive.
use crate::{AppError, AppResult};
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
pub const MAX_ARTICLE_HTML: usize = 4 * 1024 * 1024;
pub fn image_url(value:&str)->AppResult<String>{
    let invalid=||AppError::new("ARTICLE_IMAGE_URL_INVALID","文章图片地址不属于微信图片服务");
    let mut url=url::Url::parse(value).map_err(|_|invalid())?;
    if url.scheme()!="https"||!matches!(url.host_str(),Some("mmbiz.qpic.cn"|"mmbiz.qlogo.cn"))||url.port().is_some()||!url.username().is_empty()||url.password().is_some()||value.len()>4096{return Err(invalid());}
    url.set_fragment(None);Ok(url.into())
}
pub struct ArticleImageBytes {pub bytes:zeroize::Zeroizing<Vec<u8>>,pub mime:String,pub extension:String}
pub fn canonical_url(value: &str) -> AppResult<String> {
    let invalid = || AppError::new("INVALID_ARGUMENT", "请输入有效的微信公众号文章链接");
    if value.len() > 4096 {
        return Err(invalid());
    }
    let mut url = url::Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("mp.weixin.qq.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    if let Some(slug) = url.path().strip_prefix("/s/") {
        if slug.is_empty()
            || !slug
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid());
        }
        url.set_query(None);
    } else if url.path() == "/s" {
        let mut fields = std::collections::BTreeMap::new();
        for (k, v) in url.query_pairs() {
            if matches!(k.as_ref(), "__biz" | "mid" | "idx" | "sn")
                && fields.insert(k.to_string(), v.to_string()).is_some()
            {
                return Err(invalid());
            }
        }
        if fields.len() != 4
            || fields.values().any(|v| v.is_empty() || v.len() > 256)
            || !fields["mid"].bytes().all(|b| b.is_ascii_digit())
            || !fields["idx"].bytes().all(|b| b.is_ascii_digit())
            || fields["sn"].len() != 32
            || !fields["sn"].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(fields);
    } else {
        return Err(invalid());
    }
    url.set_fragment(None);
    Ok(url.into())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Article {
    pub url: String,
    pub title: String,
    pub author: String,
    pub published: String,
    pub text: String,
    pub images: Vec<String>,
    pub fetched_at: i64,
    pub source_mode: String,
}
impl Article {
    pub fn valid(&self) -> bool {
        canonical_url(&self.url).is_ok_and(|u| u == self.url)
            && !self.title.trim().is_empty()
            && self.title.len() <= 4096
            && self.author.len() <= 4096
            && self.published.len() <= 256
            && (!self.text.trim().is_empty() || !self.images.is_empty())
            && self.text.len() <= MAX_ARTICLE_HTML
            && self.images.len() <= 2048
            && self
                .images
                .iter()
                .all(|v| v.len() <= 4096 && v.starts_with("https://"))
            && matches!(
                self.source_mode.as_str(),
                "public_web" | "native_authenticated"
            )
    }
}
#[derive(Default)]
struct Projection {
    stack: Vec<(String, String)>,
    body_depth: Option<usize>,
    blocked: usize,
    title: String,
    heading: String,
    author: String,
    published: String,
    text: String,
    images: Vec<String>,
    seen_body: bool,
    invalid: bool,
}
struct Sink(RefCell<Projection>);
impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut p = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind == TagKind::EndTag {
                    if let Some(at) = p.stack.iter().rposition(|(n, _)| n == name) {
                        p.stack.truncate(at);
                        p.blocked = p
                            .stack
                            .iter()
                            .filter(|(n, _)| {
                                matches!(
                                    n.as_str(),
                                    "script"
                                        | "style"
                                        | "noscript"
                                        | "template"
                                        | "iframe"
                                        | "object"
                                        | "noembed"
                                        | "noframes"
                                )
                            })
                            .count();
                        if p.body_depth.is_some_and(|depth| at < depth) {
                            p.body_depth = None;
                        }
                    }
                    if p.body_depth.is_some()
                        && matches!(
                            name,
                            "p" | "section" | "div" | "li" | "h1" | "h2" | "h3" | "blockquote"
                        )
                        && !p.text.ends_with('\n')
                    {
                        p.text.push('\n');
                    }
                    return TokenSinkResult::Continue;
                }
                let attr = |key: &str| {
                    tag.attrs
                        .iter()
                        .find(|a| a.name.local.as_ref() == key)
                        .map(|a| a.value.to_string())
                        .unwrap_or_default()
                };
                let id = attr("id");
                let blocked = matches!(
                    name,
                    "script"
                        | "style"
                        | "noscript"
                        | "template"
                        | "iframe"
                        | "object"
                        | "noembed"
                        | "noframes"
                );
                if p.blocked == 0 && name == "meta" && attr("property") == "og:title" {
                    p.title = attr("content");
                }
                if p.blocked == 0 && p.body_depth.is_some() {
                    if name == "br" {
                        p.text.push('\n');
                    }
                    if name == "img" {
                        let mut url = attr("data-src");
                        if url.is_empty() {
                            url = attr("src");
                        }
                        if url.starts_with("//") {
                            url = format!("https:{url}");
                        }
                        if url.starts_with("https://")
                            && url.len() <= 4096
                            && !p.images.contains(&url)
                        {
                            p.images.push(url);
                        }
                    }
                }
                if !matches!(
                    name,
                    "area"
                        | "base"
                        | "br"
                        | "col"
                        | "embed"
                        | "hr"
                        | "img"
                        | "input"
                        | "link"
                        | "meta"
                        | "param"
                        | "source"
                        | "track"
                        | "wbr"
                ) {
                    if p.stack.len() >= 512 {
                        p.invalid = true;
                        return TokenSinkResult::Continue;
                    }
                    p.stack.push((name.into(), id.clone()));
                    if blocked {
                        p.blocked += 1;
                    }
                    if p.blocked == 0 && id == "js_content" {
                        if p.seen_body {
                            p.invalid = true;
                        }
                        p.seen_body = true;
                        p.body_depth = Some(p.stack.len());
                    }
                }
                if name == "script" {
                    return TokenSinkResult::RawData(RawKind::ScriptData);
                }
                if matches!(name, "style" | "iframe" | "noembed" | "noframes") {
                    return TokenSinkResult::RawData(RawKind::Rawtext);
                }
                if matches!(name, "title" | "textarea") {
                    return TokenSinkResult::RawData(RawKind::Rcdata);
                }
            }
            Token::CharacterTokens(text) if p.blocked == 0 => {
                let title = p.stack.iter().any(|(_, id)| id == "activity-name");
                let author = p.stack.iter().any(|(_, id)| id == "js_name");
                let date = p.stack.iter().any(|(_, id)| id == "publish_time");
                if p.body_depth.is_some() {
                    p.text.push_str(&text);
                } else if author {
                    p.author.push_str(&text);
                } else if date {
                    p.published.push_str(&text);
                } else if title {
                    p.heading.push_str(&text);
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}
pub fn parse_article(url: &str, html: &str, mode: &str, now: i64) -> AppResult<Article> {
    if html.len() > MAX_ARTICLE_HTML || html.contains('\0') {
        return Err(AppError::new("ARTICLE_TOO_LARGE", "文章超过读取上限"));
    }
    let input = BufferQueue::default();
    input.push_back(html.into());
    let tokenizer = Tokenizer::new(
        Sink(RefCell::new(Projection::default())),
        Default::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let p = tokenizer.sink.0.borrow();
    if !p.seen_body || p.invalid {
        return Err(AppError::new(
            "ARTICLE_CONTENT_UNAVAILABLE",
            "页面未包含可读取的文章正文；可能需要微信登录或文章已失效",
        ));
    }
    let article = Article {
        url: canonical_url(url)?,
        title: if p.title.trim().is_empty() {
            p.heading.trim()
        } else {
            p.title.trim()
        }
        .into(),
        author: p.author.trim().into(),
        published: p.published.trim().into(),
        text: p
            .text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        images: p.images.clone(),
        fetched_at: now,
        source_mode: mode.into(),
    };
    if !article.valid() {
        return Err(AppError::new(
            "ARTICLE_CONTENT_INVALID",
            "文章字段或正文无效",
        ));
    }
    Ok(article)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn article_extracts_entities_and_ignores_executable_content() {
        let html = r#"<meta property="og:title" content="自动化 &amp; 测试"><span id="js_name">作者</span><div id="js_content"><p>第一 &lt;段&gt;</p><script>"<p>不能出现</p>"</script><style>.private{}</style><p>第二段<img data-src="https://mmbiz.qpic.cn/test"></p></div><footer>其他内容</footer>"#;
        let a = parse_article("https://mp.weixin.qq.com/s/test", html, "public_web", 1).unwrap();
        assert_eq!(a.title, "自动化 & 测试");
        assert_eq!(a.author, "作者");
        assert_eq!(a.text, "第一 <段>\n第二段");
        assert_eq!(a.images.len(), 1);
        assert!(
            parse_article(
                "https://mp.weixin.qq.com/s/test",
                "<h1>登录</h1>",
                "public_web",
                1
            )
            .is_err()
        );
    }
}

/// Validate every frame before retaining a public WebP image. The browser can
/// play the original animation; it is never flattened into a misleading still.
pub fn validate_webp(bytes:&[u8],cancel:&std::sync::atomic::AtomicBool)->AppResult<()> {
    use std::sync::atomic::Ordering;
    let invalid=||AppError::new("ARTICLE_WEBP_INVALID","WebP图片不完整或超过8MiB、4096边长、120帧、60秒或3200万解码像素");
    if bytes.len()<20||bytes.len()>crate::image::MAX_IMAGE_BYTES||bytes.get(..4)!=Some(b"RIFF")||bytes.get(8..12)!=Some(b"WEBP")||u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize+8!=bytes.len(){return Err(invalid());}
    let mut decoder=image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|_|invalid())?;
    decoder.set_memory_limit(32*1024*1024);let (w,h)=decoder.dimensions();let frames=decoder.num_frames().max(1);
    if w==0||h==0||w>4096||h>4096||u64::from(w)*u64::from(h)>crate::image::MAX_IMAGE_PIXELS||frames>120||u64::from(w)*u64::from(h)*u64::from(frames)>32_000_000{return Err(invalid());}
    let size=decoder.output_buffer_size().filter(|n|*n<=16*1024*1024).ok_or_else(invalid)?;
    let mut buffer=zeroize::Zeroizing::new(vec![0u8;size]);let mut duration=0u64;
    for _ in 0..frames {
        if cancel.load(Ordering::SeqCst){return Err(AppError::new("QUERY_CANCELED","图片读取已取消"));}
        if decoder.is_animated(){duration+=u64::from(decoder.read_frame(&mut buffer).map_err(|_|invalid())?);if duration>60000{return Err(invalid());}}
        else{decoder.read_image(&mut buffer).map_err(|_|invalid())?;}
    }Ok(())
}
