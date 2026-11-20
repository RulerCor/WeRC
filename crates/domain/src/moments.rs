//! Whitelisted local timeline fields. No raw XML, URLs or media credentials.
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

pub const LIVE_MOMENT_MAX_ROWS: usize = 30;
pub const LIVE_MOMENT_SCAN_ROWS: usize = 300;
pub const LIVE_MOMENT_SCAN_BYTES: usize = 32 * 1024 * 1024;
pub const LIVE_MOMENT_MAX_BYTES: usize = 4 * 1024 * 1024;
#[path = "moments/details.rs"]
mod details;
pub use details::*;
pub const MOMENT_XML_MAX_BYTES: usize = 65536;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMomentAnchor {
    /// SQLite stores the same unsigned bit pattern in a signed INTEGER.
    pub id: String,
    pub row_id: String,
    pub username: String,
}
impl LiveMomentAnchor {
    pub fn key(&self) -> Option<(u64, i64)> {
        let id = self.id.parse::<u64>().ok()?;
        let row = self.row_id.parse::<i64>().ok()?;
        (id != 0
            && id.to_string() == self.id
            && row.to_string() == self.row_id
            && !self.username.is_empty()
            && self.username.len() <= 256
            && !self.username.chars().any(char::is_control))
        .then_some((id, row))
    }
}
impl Drop for LiveMomentAnchor {
    fn drop(&mut self) {
        self.username.zeroize();
        self.id.zeroize();
        self.row_id.zeroize();
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMomentContinuation {
    pub source_fingerprint: String,
    pub anchor: LiveMomentAnchor,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentContent {
    #[serde(default)]
    pub details: MomentDetails,
    pub timestamp: i64,
    pub text: String,
    pub content_type: Option<u32>,
    pub media_count: u32,
    pub cached_like_count: u32,
    pub cached_comment_count: u32,
}
impl MomentContent {
    pub fn valid(&self) -> bool {
        (0..=253402300799).contains(&self.timestamp)
            && self.text.len() <= 16384
            && !self
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            && self.media_count <= 2048
            && self.cached_like_count <= 2048
            && self.cached_comment_count <= 2048
            && self.details.valid()
    }
}
impl Drop for MomentContent {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMoment {
    pub anchor: LiveMomentAnchor,
    pub content: Option<MomentContent>,
    pub content_status: String,
}
impl LiveMoment {
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some()
            && match (self.content_status.as_str(), &self.content) {
                ("parsed", Some(c)) => c.valid(),
                ("missing" | "oversized" | "unsupported" | "invalid", None) => true,
                _ => false,
            }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMomentWindow {
    pub schema_version: u32,
    pub rows: Vec<LiveMoment>,
    pub has_more: bool,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveMomentWindow {
    pub fn valid(&self, after: Option<&LiveMomentAnchor>) -> bool {
        self.valid_with_limit(after, LIVE_MOMENT_MAX_ROWS)
    }
    pub fn valid_with_limit(&self, after: Option<&LiveMomentAnchor>, limit: usize) -> bool {
        if !matches!(limit, LIVE_MOMENT_MAX_ROWS | LIVE_MOMENT_SCAN_ROWS)
            || self.schema_version != 1
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > limit
            || (self.has_more && self.rows.len() != limit)
            || after.is_some_and(|a| a.key().is_none())
        {
            return false;
        }
        let mut previous = after.and_then(LiveMomentAnchor::key);
        for row in &self.rows {
            if !row.valid() {
                return false;
            }
            let key = row.anchor.key().expect("validated anchor");
            if previous.is_some_and(|p| key >= p) {
                return false;
            }
            previous = Some(key);
        }
        true
    }
}

/// Reject ambiguous identity fields; ignore all unlisted payload values.
pub fn parse_content(xml: &str, anchor: &LiveMomentAnchor) -> Result<MomentContent, &'static str> {
    use quick_xml::{Reader, events::Event};
    const INVALID: &str = "LIVE_MOMENT_CONTENT_INVALID";
    let (expected_id, _) = anchor.key().ok_or(INVALID)?;
    if xml.len() > MOMENT_XML_MAX_BYTES || xml.contains('\0') {
        return Err(INVALID);
    }
    const FIELDS: [&str; 5] = [
        "SnsDataItem/TimelineObject/id",
        "SnsDataItem/TimelineObject/username",
        "SnsDataItem/TimelineObject/createTime",
        "SnsDataItem/TimelineObject/contentDesc",
        "SnsDataItem/TimelineObject/ContentObject/contentStyle",
    ];
    const CONTAINERS: [&str; 6] = [
        "SnsDataItem/TimelineObject",
        "SnsDataItem/LocalExtraInfo",
        "SnsDataItem/TimelineObject/ContentObject",
        "SnsDataItem/TimelineObject/ContentObject/mediaList",
        "SnsDataItem/LocalExtraInfo/like_user_list",
        "SnsDataItem/LocalExtraInfo/comment_user_list",
    ];
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<String>::new();
    let mut values: [Option<Zeroizing<String>>; 5] = Default::default();
    let mut seen = [false; 6];
    let (mut roots, mut nodes, mut declaration) = (0, 0, false);
    let (mut media, mut likes, mut comments) = (0, 0, 0);
    loop {
        match reader.read_event().map_err(|_| INVALID)? {
            Event::Start(e) => {
                nodes += 1;
                let name = e.name().as_ref().to_owned();
                if nodes > 2048 || stack.len() >= 32 || name.len() > 64 {
                    return Err("LIVE_MOMENT_XML_LIMIT");
                }
                if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    return Err("LIVE_MOMENT_XML_NAME");
                }
                if FIELDS.contains(&stack.join("/").as_str()) {
                    return Err("LIVE_MOMENT_FIELD_NESTED");
                }
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || name != "SnsDataItem" {
                        return Err(INVALID);
                    }
                }
                for (i, a) in e.attributes().enumerate() {
                    if i >= 64 {
                        return Err(INVALID);
                    }
                    a.map_err(|_| INVALID)?;
                }
                stack.push(name);
                let path = stack.join("/");
                if let Some(i) = CONTAINERS.iter().position(|p| *p == path) {
                    if seen[i] {
                        return Err("LIVE_MOMENT_CONTAINER_DUPLICATE");
                    }
                    seen[i] = true;
                }
                if let Some(i) = FIELDS.iter().position(|p| *p == path) {
                    if values[i].is_some() {
                        return Err("LIVE_MOMENT_FIELD_DUPLICATE");
                    }
                    values[i] = Some(Zeroizing::new(String::new()));
                }
                match path.as_str() {
                    "SnsDataItem/TimelineObject/ContentObject/mediaList/media" => media += 1,
                    "SnsDataItem/LocalExtraInfo/like_user_list/user_comment" => likes += 1,
                    "SnsDataItem/LocalExtraInfo/comment_user_list/user_comment" => comments += 1,
                    _ => {}
                }
            }
            Event::End(e) => {
                if stack.pop().as_deref() != Some(e.name().as_ref()) {
                    return Err(INVALID);
                }
            }
            Event::Text(t) => append(&stack, &FIELDS, &mut values, &t.xml10_content())?,
            Event::CData(t) if !stack.is_empty() => {
                append(&stack, &FIELDS, &mut values, &t.xml10_content())?
            }
            Event::GeneralRef(r) if !stack.is_empty() => {
                let named = r.xml10_content();
                let c = match named.as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => r.resolve_char_ref().map_err(|_| INVALID)?.ok_or(INVALID)?,
                };
                append(&stack, &FIELDS, &mut values, &c.to_string())?;
            }
            Event::Comment(_) => {}
            Event::Decl(_) if roots == 0 && !declaration => {
                declaration = true;
            }
            Event::Eof => break,
            _ => return Err(INVALID),
        }
    }
    if roots != 1 || !stack.is_empty() {
        return Err(INVALID);
    }
    let [id, author, time, body, kind] = values;
    if id.as_ref().and_then(|v| v.trim().parse::<u64>().ok()) != Some(expected_id)
        || author.as_ref().map(|v| v.as_str()) != Some(anchor.username.as_str())
    {
        return Err("LIVE_MOMENT_IDENTITY_MISMATCH");
    }
    let result = MomentContent {
        details: parse_details(xml).map_err(|_| "LIVE_MOMENT_DETAILS_INVALID")?,
        timestamp: time
            .ok_or("LIVE_MOMENT_TIME_MISSING")?
            .trim()
            .parse()
            .map_err(|_| "LIVE_MOMENT_TIME_INVALID")?,
        text: body.map(|v| v.to_string()).unwrap_or_default(),
        content_type: kind
            .map(|v| v.trim().parse())
            .transpose()
            .map_err(|_| INVALID)?,
        media_count: media,
        cached_like_count: likes,
        cached_comment_count: comments,
    };
    if !result.valid() {
        return Err("LIVE_MOMENT_FIELDS_INVALID");
    }
    Ok(result)
}
fn append(
    stack: &[String],
    fields: &[&str; 5],
    values: &mut [Option<Zeroizing<String>>; 5],
    text: &str,
) -> Result<(), &'static str> {
    if stack.is_empty() && !text.trim().is_empty() {
        return Err("LIVE_MOMENT_TEXT_INVALID");
    }
    if let Some(i) = fields.iter().position(|p| *p == stack.join("/")) {
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("LIVE_MOMENT_TEXT_INVALID");
        }
        let value = values[i].as_mut().ok_or("LIVE_MOMENT_CONTENT_INVALID")?;
        if value.len() + text.len() > [20, 256, 20, 16384, 10][i] {
            return Err("LIVE_MOMENT_FIELD_LIMIT");
        }
        value.push_str(text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn anchor() -> LiveMomentAnchor {
        LiveMomentAnchor {
            id: u64::MAX.to_string(),
            row_id: "-1".into(),
            username: "owned-author".into(),
        }
    }
    fn xml(body: &str) -> String {
        format!(
            "<SnsDataItem><TimelineObject><id>{}</id><username>owned-author</username><createTime>1700000000</createTime><contentDesc>{body}</contentDesc><ContentObject><contentStyle>1</contentStyle><mediaList><media><url>private-url</url><key>secret-key</key></media></mediaList></ContentObject></TimelineObject><LocalExtraInfo><like_user_list><user_comment><username>hidden</username></user_comment></like_user_list></LocalExtraInfo></SnsDataItem>",
            u64::MAX
        )
    }
    #[test]
    fn whitelist_signed_identity_unicode_and_counts() {
        let parsed = parse_content(
            &xml("<![CDATA[自动化测试 <>]]>&amp;&#x1F600;&#10;"),
            &anchor(),
        )
        .unwrap();
        assert_eq!(parsed.text, "自动化测试 <>&😀\n");
        assert_eq!(
            (
                parsed.media_count,
                parsed.cached_like_count,
                parsed.cached_comment_count
            ),
            (1, 1, 0)
        );
        let json = serde_json::to_string(&parsed).unwrap();
        for private in ["private-url", "secret-key", "owned-author"] {
            assert!(!json.contains(private));
        }
    }
    #[test]
    fn ambiguous_truncated_mismatched_and_expansive_documents_rejected() {
        for bad in [
            xml("ok").replace(
                "</username>",
                "</username><username>owned-author</username>",
            ),
            xml("ok").replace("owned-author", "different"),
            xml("ok").replace(&u64::MAX.to_string(), "1"),
            xml("ok").replace("</SnsDataItem>", ""),
            format!("{}{}", xml("ok"), xml("ok")),
            format!("<!DOCTYPE x [<!ENTITY boom 'payload'>]>{}", xml("&boom;")),
            xml("<nested>bad</nested>"),
            xml("&#0;"),
            xml("&unknown;"),
            xml(&"中".repeat(5462)),
            xml("ok").replace(
                "<LocalExtraInfo>",
                &format!("<LocalExtraInfo>{}", "<x/>".repeat(2049)),
            ),
        ] {
            assert!(parse_content(&bad, &anchor()).is_err());
        }
    }
    #[test]
    fn ignored_extension_text_does_not_hide_verified_body() {
        let content = xml("自动化测试").replace(
            "<LocalExtraInfo>",
            "<LocalExtraInfo><private_extension><![CDATA[\u{0085}]]></private_extension>",
        );
        assert_eq!(
            parse_content(&content, &anchor()).unwrap().text,
            "自动化测试"
        );
        assert!(parse_content(&xml("bad\u{0085}"), &anchor()).is_err());
    }
    #[test]
    fn ordering_is_unsigned_and_row_ids_are_exact() {
        let make = |id: u64, row: i64| LiveMoment {
            anchor: LiveMomentAnchor {
                id: id.to_string(),
                row_id: row.to_string(),
                username: "owned".into(),
            },
            content: None,
            content_status: "missing".into(),
        };
        let mut w = LiveMomentWindow {
            schema_version: 1,
            rows: vec![make(u64::MAX, 2), make(u64::MAX, 1), make(1, 9)],
            has_more: false,
            read_only: true,
            readonly_shm: true,
        };
        assert!(w.valid(None));
        w.rows.swap(0, 1);
        assert!(!w.valid(None));
        let mut a = anchor();
        a.id = "01".into();
        assert!(a.key().is_none());
    }
}

/// Filters apply to the entire local timeline cache, not only the displayed page.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentQuery {
    #[serde(default)]
    pub text: String,
    pub author: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
    pub content_type: Option<u32>,
}
impl MomentQuery {
    pub fn valid(&self) -> bool {
        self.text.len() <= 512
            && !self.text.chars().any(char::is_control)
            && self
                .author
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            && self.from.is_none_or(|n| (0..253402300800).contains(&n))
            && self.until.is_none_or(|n| (1..=253402300800).contains(&n))
            && !matches!((self.from,self.until),(Some(a),Some(b)) if a>=b)
    }
    pub fn matches(&self, row: &LiveMoment) -> bool {
        if self
            .author
            .as_ref()
            .is_some_and(|a| a != &row.anchor.username)
        {
            return false;
        }
        let Some(c) = &row.content else {
            return self.text.is_empty()
                && self.from.is_none()
                && self.until.is_none()
                && self.content_type.is_none();
        };
        !self.from.is_some_and(|n| c.timestamp < n)
            && !self.until.is_some_and(|n| c.timestamp >= n)
            && !self.content_type.is_some_and(|t| c.content_type != Some(t))
            && (self.text.is_empty()
                || [
                    c.text.as_str(),
                    c.details.title.as_str(),
                    c.details.description.as_str(),
                ]
                .iter()
                .any(|v| v.to_lowercase().contains(&self.text.to_lowercase())))
    }
}

#[path = "moments/media.rs"]
mod media;
pub use media::*;
#[path = "moments/isaac.rs"]
pub mod isaac;
