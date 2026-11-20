//! Display-only projection of the locally observed SNS XML fields.
use super::*;
use quick_xml::{Reader, events::Event};
use std::collections::BTreeMap;
const INVALID: &str = "LIVE_MOMENT_CONTENT_INVALID";
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentDetails {
    pub title: String,
    pub description: String,
    pub link: Option<String>,
    pub source_name: String,
    pub location: BTreeMap<String, String>,
    pub media: Vec<MomentMediaInfo>,
    pub likes: Vec<MomentInteraction>,
    pub comments: Vec<MomentInteraction>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentMediaInfo {
    pub slot: usize,
    pub kind: Option<u32>,
    pub title: String,
    pub description: String,
    pub width: Option<String>,
    pub height: Option<String>,
    pub duration: Option<String>,
    pub live_photo: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentInteraction {
    pub username: String,
    pub nickname: String,
    pub text: String,
    pub timestamp: Option<i64>,
    pub id: Option<String>,
    pub id64: Option<String>,
    pub reply_id: Option<String>,
    pub reply_id64: Option<String>,
    pub reply_username: String,
    pub deleted: Option<bool>,
    pub content_type: Option<u32>,
    pub image_count: Option<u32>,
    pub emoji_count: Option<u32>,
}
impl MomentDetails {
    pub fn valid(&self) -> bool {
        self.media.len() <= 256
            && self.likes.len() <= 2048
            && self.comments.len() <= 2048
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= 65536)
            && self
                .link
                .as_ref()
                .is_none_or(|u| display_link(u).as_ref() == Some(u))
    }
}
impl Drop for MomentDetails {
    fn drop(&mut self) {
        self.title.zeroize();
        self.description.zeroize();
        self.source_name.zeroize();
        self.link.zeroize();
        for v in self.location.values_mut() {
            v.zeroize();
        }
    }
}
impl Drop for MomentInteraction {
    fn drop(&mut self) {
        self.username.zeroize();
        self.nickname.zeroize();
        self.text.zeroize();
        self.reply_username.zeroize();
        self.id.zeroize();
        self.id64.zeroize();
        self.reply_id.zeroize();
        self.reply_id64.zeroize();
    }
}
impl Drop for MomentMediaInfo {
    fn drop(&mut self) {
        self.title.zeroize();
        self.description.zeroize();
    }
}
pub(super) struct Node {
    pub(super) name: String,
    pub(super) text: Zeroizing<String>,
    pub(super) attributes: BTreeMap<String, String>,
    pub(super) children: Vec<Node>,
}
impl Node {
    pub(super) fn child(&self, name: &str) -> Result<Option<&Self>, &'static str> {
        let mut it = self.children.iter().filter(|n| n.name == name);
        let result = it.next();
        if it.next().is_some() {
            Err(INVALID)
        } else {
            Ok(result)
        }
    }
    pub(super) fn value(&self, name: &str) -> Result<&str, &'static str> {
        match self.child(name)? {
            Some(n) if n.children.is_empty() => Ok(&n.text),
            None => Ok(""),
            _ => Err(INVALID),
        }
    }
}
fn display_link(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()
        && !authority.contains('@')
        && !s.contains('\\')
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
        && s.len() <= 4096)
        .then(|| s.into())
}
fn number(s: &str) -> Option<String> {
    let v = s.trim().parse::<u64>().ok()?;
    (v != 0).then(|| v.to_string())
}
fn append(stack: &mut [Node], text: &str) -> Result<(), &'static str> {
    if let Some(node) = stack.last_mut() {
        if node.text.len() + text.len() > 65536 {
            return Err(INVALID);
        };
        node.text.push_str(text);
    }
    Ok(())
}
pub(super) fn tree(xml: &str) -> Result<Node, &'static str> {
    // parse_content already checks the document identity and resource limits.
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<Node>::new();
    let mut root = None;
    loop {
        match reader.read_event().map_err(|_| INVALID)? {
            Event::Start(e) => {
                let mut attributes = BTreeMap::new();
                let name = e.name().as_ref().to_owned();
                if matches!(name.as_str(), "location" | "size" | "url" | "thumb" | "enc") {
                    for attr in e.attributes() {
                        let a = attr.map_err(|_| INVALID)?;
                        if matches!(
                            a.key.as_ref(),
                            "key"
                                | "token"
                                | "md5"
                                | "videomd5"
                                | "enc_idx"
                                | "type"
                                | "totalSize"
                                | "city"
                                | "country"
                                | "poiName"
                                | "poiAddress"
                                | "poiAddressName"
                                | "latitude"
                                | "longitude"
                                | "width"
                                | "height"
                        ) {
                            let value = a
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map_err(|_| INVALID)?;
                            if value.len() > 4096 {
                                return Err(INVALID);
                            }
                            attributes.insert(a.key.as_ref().to_owned(), value.into_owned());
                        }
                    }
                }
                stack.push(Node {
                    name,
                    text: Zeroizing::new(String::new()),
                    attributes,
                    children: vec![],
                });
            }
            Event::Text(e) => append(&mut stack, &e.xml10_content())?,
            Event::CData(e) => append(&mut stack, &e.xml10_content())?,
            Event::GeneralRef(e) => {
                let name = e.xml10_content();
                let c = match name.as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => e.resolve_char_ref().map_err(|_| INVALID)?.ok_or(INVALID)?,
                };
                append(&mut stack, &c.to_string())?;
            }
            Event::End(_) => {
                let n = stack.pop().ok_or(INVALID)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                } else {
                    root = Some(n);
                }
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) => {}
            _ => return Err(INVALID),
        }
    }
    root.ok_or(INVALID)
}
pub(super) fn parse_details(xml: &str) -> Result<MomentDetails, &'static str> {
    let root = tree(xml)?;
    let timeline = root.child("TimelineObject")?.ok_or(INVALID)?;
    let mut result = MomentDetails::default();
    result.source_name = timeline.value("sourceNickName")?.into();
    if let Some(location) = timeline.child("location")? {
        result.location = location
            .attributes
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    if let Some(content) = timeline.child("ContentObject")? {
        result.title = content.value("title")?.into();
        result.description = content.value("description")?.into();
        result.link = display_link(content.value("contentUrl")?);
        if let Some(list) = content.child("mediaList")? {
            for (slot, m) in list
                .children
                .iter()
                .filter(|n| n.name == "media")
                .enumerate()
            {
                let size = m.child("size")?;
                result.media.push(MomentMediaInfo {
                    slot,
                    kind: m.value("type")?.parse().ok(),
                    title: m.value("title")?.into(),
                    description: m.value("description")?.into(),
                    width: size.and_then(|n| n.attributes.get("width")).cloned(),
                    height: size.and_then(|n| n.attributes.get("height")).cloned(),
                    duration: (!m.value("videoDuration")?.is_empty())
                        .then(|| m.value("videoDuration").unwrap().into()),
                    live_photo: m.child("LivePhoto")?.is_some(),
                });
            }
        }
    }
    if let Some(extra) = root.child("LocalExtraInfo")? {
        for (name, out) in [
            ("like_user_list", &mut result.likes),
            ("comment_user_list", &mut result.comments),
        ] {
            if let Some(list) = extra.child(name)? {
                for n in list.children.iter().filter(|n| n.name == "user_comment") {
                    let timestamp = n
                        .value("create_time")?
                        .parse::<i64>()
                        .ok()
                        .filter(|t| (0..253402300800).contains(t));
                    out.push(MomentInteraction {
                        username: n.value("username")?.into(),
                        nickname: n.value("nickname")?.into(),
                        text: n.value("content")?.into(),
                        timestamp,
                        id: number(n.value("comment_id")?),
                        id64: number(n.value("comment_64id")?),
                        reply_id: number(n.value("ref_comment_id")?),
                        reply_id64: number(n.value("ref_comment_64id")?),
                        reply_username: n.value("ref_username")?.into(),
                        deleted: match n.value("b_deleted")? {
                            "0" => Some(false),
                            "1" => Some(true),
                            _ => None,
                        },
                        content_type: n.value("comment_content_type")?.parse().ok(),
                        image_count: n.value("comment_imageinfo_count")?.parse().ok(),
                        emoji_count: n.value("comment_emojiinfo_count")?.parse().ok(),
                    });
                }
            }
        }
    }
    if !result.valid() {
        return Err(INVALID);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn details_preserve_large_reply_ids_location_and_live_photo_without_media_credentials() {
        let anchor = LiveMomentAnchor {
            id: "18446744073709551615".into(),
            row_id: "1".into(),
            username: "owned".into(),
        };
        let xml = r#"<SnsDataItem><TimelineObject><id>18446744073709551615</id><username>owned</username><createTime>1700000000</createTime><contentDesc>自动化测试</contentDesc><location city="测试 &amp; 城市" poiName="广场"/><ContentObject><title>分享标题</title><contentUrl>https://example.org/a</contentUrl><mediaList><media><type>2</type><url key="secret-key">private-url</url><size width="640" height="360"/><LivePhoto><liveMedia><url>private-live-url</url></liveMedia></LivePhoto></media></mediaList></ContentObject></TimelineObject><LocalExtraInfo><comment_user_list><user_comment><username>other</username><nickname>测试者</nickname><content>回复内容 &amp; 更多</content><comment_64id>18446744073709551614</comment_64id><ref_comment_64id>18446744073709551613</ref_comment_64id><ref_username>owned</ref_username><b_deleted>1</b_deleted></user_comment></comment_user_list></LocalExtraInfo></SnsDataItem>"#;
        let content = parse_content(xml, &anchor).unwrap();
        let d = &content.details;
        assert_eq!(d.location["city"], "测试 & 城市");
        assert!(d.media[0].live_photo);
        assert_eq!(d.comments[0].id64.as_deref(), Some("18446744073709551614"));
        assert_eq!(
            d.comments[0].reply_id64.as_deref(),
            Some("18446744073709551613")
        );
        assert_eq!(d.comments[0].deleted, Some(true));
        assert_eq!(d.comments[0].text, "回复内容 & 更多");
        let public = serde_json::to_string(&content).unwrap();
        for secret in ["secret-key", "private-url", "private-live-url"] {
            assert!(!public.contains(secret));
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        for v in self.attributes.values_mut() {
            v.zeroize();
        }
    }
}
