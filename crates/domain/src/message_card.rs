//! Display-only fields from bounded contact, location, and application XML. No transport credentials or HTML.
use quick_xml::{Reader, events::Event};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};
pub mod forward_file;
pub mod forward_media;
mod record_detail;
mod record_tree;
pub mod records;

#[derive(Serialize)]
#[serde(untagged)]
pub enum DisplayContent {
    Card(MessageCard),
    Record { kind: &'static str, detail: records::RecordDetail },
}
/// Shared display projection for source conversations and persisted observations.
pub fn display(kind:u32,body:&str) -> Option<DisplayContent> {
    if !matches!(kind,37|42|48|49|50|10000|10002) {return None;}
    match parse_for_type(kind,body).ok()? {
        Some(card)=>Some(DisplayContent::Card(card)),
        None=>Some(DisplayContent::Record{kind:"record",detail:records::extract(kind,body).ok()??}),
    }
}

const INVALID: &str = "MESSAGE_CARD_INVALID";
const MAX_XML: usize = 65536;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageCard {
    Contact {
        username: String,
        nickname: String,
    },
    Location {
        latitude: String,
        longitude: String,
        label: String,
        place_name: String,
    },
    AppShare {
        category: ShareCategory,
        title: String,
        description: String,
        source_name: String,
        url: Option<String>,
    },
    Link {
        title: String,
        description: String,
        url: Option<String>,
    },
    Quote {
        text: String,
        author: String,
        quoted_type: u32,
        quoted_text: Option<String>,
    },
    Forward {
        title: String,
        entries: Vec<RecordEntry>,
        omitted: usize,
    },
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareCategory {
    Music,
    Video,
    MiniProgram,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RecordEntry {
    pub author: String,
    pub timestamp: String,
    pub data_type: u32,
    pub text: Option<String>,
    /// Only a capability hint. Credentials remain in the authenticated source.
    pub image_available: bool,
    pub file_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Box<MessageCard>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nested: Option<NestedRecord>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct NestedRecord {
    pub entries: Vec<RecordEntry>,
    pub omitted: usize,
    pub depth_limited: bool,
}
impl Drop for RecordEntry {
    fn drop(&mut self) {
        self.author.zeroize();
        self.timestamp.zeroize();
        self.text.zeroize();
    }
}
impl Drop for MessageCard {
    fn drop(&mut self) {
        match self {
            Self::Contact { username, nickname } => {
                username.zeroize();
                nickname.zeroize();
            }
            Self::Location {
                latitude,
                longitude,
                label,
                place_name,
            } => {
                latitude.zeroize();
                longitude.zeroize();
                label.zeroize();
                place_name.zeroize();
            }
            Self::AppShare {
                title,
                description,
                source_name,
                url,
                ..
            } => {
                title.zeroize();
                description.zeroize();
                source_name.zeroize();
                url.zeroize();
            }
            Self::Link {
                title,
                description,
                url,
            } => {
                title.zeroize();
                description.zeroize();
                url.zeroize();
            }
            Self::Quote {
                text,
                author,
                quoted_text,
                ..
            } => {
                text.zeroize();
                author.zeroize();
                quoted_text.zeroize();
            }
            Self::Forward { title, .. } => title.zeroize(),
        }
    }
}
struct Node {
    name: String,
    text: Zeroizing<String>,
    attributes: Vec<(String, Zeroizing<String>)>,
    data_type: Option<u32>,
    children: Vec<Node>,
}
impl Node {
    fn attribute(&self, name: &str) -> &str {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    }
    fn child(&self, name: &str) -> Result<Option<&Self>, &'static str> {
        let mut matching = self.children.iter().filter(|n| n.name == name);
        let first = matching.next();
        if matching.next().is_some() {
            return Err(INVALID);
        }
        Ok(first)
    }
    fn value(&self, name: &str) -> Result<&str, &'static str> {
        match self.child(name)? {
            Some(node) if node.children.is_empty() => Ok(node.text.as_str()),
            Some(_) => Err(INVALID),
            None => Ok(""),
        }
    }
}
fn keep_text(name: &str) -> bool {
    matches!(
        name,
        "type"
            | "feedesc"
            | "pay_memo"
            | "paysubtype"
            | "receivertitle"
            | "sendertitle"
            | "replacemsg"
            | "newmsgid"
            | "svrid"
            | "session"
            | "voipmsg"
            | "title"
            | "des"
            | "url"
            | "displayname"
            | "sourcedisplayname"
            | "content"
            | "recorditem"
            | "recordxml"
            | "datadesc"
            | "sourcename"
            | "sourcetime"
            | "cdndataurl"
            | "cdndatakey"
            | "datasize"
            | "fullmd5"
            | "datatitle"
            | "link"
            | "desc"
            | "lat"
            | "lng"
            | "label"
            | "poiname"
    )
}
fn append(stack: &mut [Node], value: &str) -> Result<(), &'static str> {
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(INVALID);
    }
    if let Some(node) = stack.last_mut() {
        if keep_text(&node.name) {
            node.text.push_str(value)
        }
    } else if !value.trim().is_empty() {
        return Err(INVALID);
    }
    Ok(())
}
fn document(xml: &str) -> Result<Node, &'static str> {
    if xml.len() > MAX_XML || xml.contains('\0') {
        return Err(INVALID);
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<Node>::new();
    let mut root = None;
    let mut nodes = 0;
    let mut declaration = false;
    loop {
        match reader.read_event().map_err(|_| INVALID)? {
            Event::Start(e) => {
                nodes += 1;
                let name = e.name().as_ref().to_owned();
                if nodes > 4096
                    || stack.len() >= 32
                    || name.len() > 64
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || (stack.is_empty() && root.is_some())
                {
                    return Err(INVALID);
                }
                let mut data_type = None;
                let mut attributes = Vec::new();
                for (i, attribute) in e.attributes().enumerate() {
                    if i >= 64 {
                        return Err(INVALID);
                    }
                    let attribute = attribute.map_err(|_| INVALID)?;
                    if (name == "msg" && matches!(attribute.key.as_ref(), "username" | "nickname"))
                        || (name == "location"
                            && matches!(attribute.key.as_ref(), "x" | "y" | "label" | "poiname"))
                    {
                        let value = attribute
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|_| INVALID)?;
                        if value.chars().any(|c| c.is_control()) {
                            return Err(INVALID);
                        }
                        attributes.push((
                            attribute.key.as_ref().to_owned(),
                            Zeroizing::new(value.into_owned()),
                        ));
                    }
                    if name == "dataitem" && attribute.key.as_ref() == "datatype" {
                        if data_type.is_some() {
                            return Err(INVALID);
                        }
                        data_type = Some(
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map_err(|_| INVALID)?
                                .parse()
                                .map_err(|_| INVALID)?,
                        );
                    }
                }
                stack.push(Node {
                    name,
                    text: Zeroizing::new(String::new()),
                    attributes,
                    data_type,
                    children: Vec::new(),
                });
            }
            Event::End(e) => {
                let node = stack.pop().ok_or(INVALID)?;
                if node.name != e.name().as_ref() {
                    return Err(INVALID);
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node)
                } else {
                    root = Some(node)
                }
            }
            Event::Text(e) => append(&mut stack, &e.xml10_content())?,
            Event::CData(e) if !stack.is_empty() => append(&mut stack, &e.xml10_content())?,
            Event::GeneralRef(e) if !stack.is_empty() => {
                let named = e.xml10_content();
                let c = match named.as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => e.resolve_char_ref().map_err(|_| INVALID)?.ok_or(INVALID)?,
                };
                append(&mut stack, &c.to_string())?;
            }
            Event::Comment(_) => {}
            Event::Decl(_) if root.is_none() && stack.is_empty() && !declaration => {
                declaration = true
            }
            Event::Eof => break,
            _ => return Err(INVALID),
        }
    }
    if !stack.is_empty() {
        return Err(INVALID);
    }
    root.ok_or(INVALID)
}
/// Display/copy only. No automatic requests, thumbnail loading, or navigation.
fn display_url(raw: &str) -> Option<String> {
    let value = raw.trim();
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if value.len() > 4096
        || authority.is_empty()
        || authority.contains('@')
        || value.contains('\\')
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    Some(value.into())
}
/// Message type comes from the authenticated row, never from untrusted XML hints.
pub fn app_subtype(message_type:u64,xml:&str)->Result<Option<u32>,&'static str>{
    if message_type&0xffffffff!=49{return Ok(None);}
    let encoded=(message_type>>32) as u32;
    if encoded!=0{return Ok(Some(encoded));}
    let root=document(xml)?;
    if root.name!="msg"{return Err(INVALID);}
    let app=root.child("appmsg")?.ok_or(INVALID)?;
    Ok(Some(app.value("type")?.trim().parse::<u32>().map_err(|_|INVALID)?))
}

pub fn parse_for_type(message_type: u32, xml: &str) -> Result<Option<MessageCard>, &'static str> {
    if message_type == 49 {
        return parse(xml);
    }
    if !matches!(message_type, 42 | 48) {
        return Ok(None);
    }
    let root = document(xml)?;
    if root.name != "msg" {
        return Err(INVALID);
    }
    if message_type == 42 {
        let username = root.attribute("username");
        if username.is_empty() || username.len() > 256 || username.chars().any(char::is_whitespace)
        {
            return Err(INVALID);
        }
        return Ok(Some(MessageCard::Contact {
            username: username.into(),
            nickname: root.attribute("nickname").into(),
        }));
    }
    let location = root.child("location")?.ok_or(INVALID)?;
    let latitude = location.attribute("x").trim();
    let longitude = location.attribute("y").trim();
    for (value, limit) in [(latitude, 90.0), (longitude, 180.0)] {
        let number = value.parse::<f64>().map_err(|_| INVALID)?;
        if !number.is_finite() || number.abs() > limit {
            return Err(INVALID);
        }
    }
    Ok(Some(MessageCard::Location {
        latitude: latitude.into(),
        longitude: longitude.into(),
        label: location.attribute("label").into(),
        place_name: location.attribute("poiname").into(),
    }))
}
/// Integrity check after selecting a causally captured outgoing local ID.
pub fn quote_matches(
    xml: &str,
    text: &str,
    server: &str,
    source_text: &str,
) -> Result<bool, &'static str> {
    let root = document(xml)?;
    if root.name != "msg" {
        return Err(INVALID);
    }
    let app = root.child("appmsg")?.ok_or(INVALID)?;
    let quote = app.child("refermsg")?.ok_or(INVALID)?;
    Ok(app.value("type")?.trim() == "57"
        && app.value("title")? == text
        && quote.value("svrid")? == server
        && quote.value("type")?.trim() == "1"
        && quote.value("content")? == source_text)
}

pub fn parse(xml: &str) -> Result<Option<MessageCard>, &'static str> {
    let root = document(xml)?;
    if root.name != "msg" {
        return Err(INVALID);
    }
    let app = root.child("appmsg")?.ok_or(INVALID)?;
    let kind = app
        .value("type")?
        .trim()
        .parse::<u32>()
        .map_err(|_| INVALID)?;
    Ok(Some(match kind {
        3 | 4 | 33 | 36 => MessageCard::AppShare {
            category: match kind {
                3 => ShareCategory::Music,
                4 => ShareCategory::Video,
                _ => ShareCategory::MiniProgram,
            },
            title: app.value("title")?.into(),
            description: app.value("des")?.into(),
            source_name: app.value("sourcedisplayname")?.into(),
            url: display_url(app.value("url")?),
        },
        5 => MessageCard::Link {
            title: app.value("title")?.into(),
            description: app.value("des")?.into(),
            url: display_url(app.value("url")?),
        },
        57 => {
            let quote = app.child("refermsg")?.ok_or(INVALID)?;
            let quoted_type = quote
                .value("type")?
                .trim()
                .parse::<u32>()
                .map_err(|_| INVALID)?;
            MessageCard::Quote {
                text: app.value("title")?.into(),
                author: quote.value("displayname")?.into(),
                quoted_type,
                quoted_text: if quoted_type == 1 {
                    Some(quote.value("content")?.into())
                } else {
                    None
                },
            }
        }
        19 => {
            let record = document(app.value("recorditem")?)?;
            let tree = record_tree::entries(&record, 0, &mut 100)?;
            MessageCard::Forward {
                title: app.value("title")?.into(),
                entries: tree.entries,
                omitted: tree.omitted,
            }
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn exact_quote_receipt_checks_original_identity_and_rejects_duplicate_fields() {
        let xml = "<msg><appmsg><type>57</type><title>reply &amp; text</title><refermsg><type>1</type><svrid>123</svrid><content>source</content></refermsg></appmsg></msg>";
        assert!(super::quote_matches(xml, "reply & text", "123", "source").unwrap());
        for (body, id, source) in [
            ("different", "123", "source"),
            ("reply & text", "124", "source"),
            ("reply & text", "123", "different"),
        ] {
            assert!(!super::quote_matches(xml, body, id, source).unwrap());
        }
        assert!(
            super::quote_matches(
                &xml.replace("<svrid>123</svrid>", "<svrid>123</svrid><svrid>124</svrid>"),
                "reply & text",
                "123",
                "source"
            )
            .is_err()
        );
    }
    use super::*;
    fn app(body: &str) -> String {
        format!("<msg><appmsg>{body}</appmsg></msg>")
    }
    #[test]
    fn link_escapes_and_no_credentials() {
        let card=parse(&app("<type>5</type><title>A &amp; B &#x1f600;</title><des><![CDATA[<b>literal</b>]]></des><url>https://example.test/a?x=1&amp;y=2</url><appattach><aeskey>secret</aeskey><cdnattachurl>private</cdnattachurl></appattach>")).unwrap().unwrap();
        assert_eq!(
            card,
            MessageCard::Link {
                title: "A & B 😀".into(),
                description: "<b>literal</b>".into(),
                url: Some("https://example.test/a?x=1&y=2".into())
            }
        );
        let public = serde_json::to_string(&card).unwrap();
        assert!(!public.contains("secret"));
        assert!(!public.contains("private"));
    }
    #[test]
    fn unsafe_urls_never_exposed() {
        for url in [
            "javascript:alert(1)",
            "file:///C:/x",
            "//example.test",
            "https://user:pass@example.test",
            "https://",
            "https://example.test\\x",
            "https://a.test/\nx",
        ] {
            assert!(display_url(url).is_none(), "{url}")
        }
    }
    #[test]
    fn quote_only_text_content_is_public() {
        let xml = app(
            "<type>57</type><title>reply</title><refermsg><type>1</type><displayname>Author</displayname><content><![CDATA[text <tag>]]></content><fromusr>private-id</fromusr></refermsg>",
        );
        assert_eq!(
            parse(&xml).unwrap().unwrap(),
            MessageCard::Quote {
                text: "reply".into(),
                author: "Author".into(),
                quoted_type: 1,
                quoted_text: Some("text <tag>".into())
            }
        );
        let xml = xml
            .replace("<type>1</type>", "<type>49</type>")
            .replace("text <tag>", "<msg><aeskey>secret</aeskey></msg>");
        assert!(
            !serde_json::to_string(&parse(&xml).unwrap())
                .unwrap()
                .contains("secret")
        );
    }
    #[test]
    fn forward_text_and_media_placeholder() {
        let xml = app(
            "<type>19</type><title>History</title><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><sourcename>A</sourcename><sourcetime>2026-11-12 08:00</sourcetime><datadesc>Hello &amp; world</datadesc></dataitem><dataitem datatype=\"2\"><datadesc>private-media-key</datadesc><cdn_dataurl>secret</cdn_dataurl></dataitem></datalist></recordinfo>]]></recorditem>",
        );
        let card = parse(&xml).unwrap().unwrap();
        if let MessageCard::Forward {
            entries, omitted, ..
        } = &card
        {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].text.as_deref(), Some("Hello & world"));
            assert!(entries[1].text.is_none());
            assert_eq!(*omitted, 0)
        } else {
            panic!("forward expected")
        }
        assert!(
            !serde_json::to_string(&card)
                .unwrap()
                .contains("private-media-key")
        );
    }
    #[test]
    fn forward_truncation_is_explicit() {
        let items = "<dataitem datatype=\"1\"><datadesc>x</datadesc></dataitem>".repeat(102);
        let xml = app(&format!(
            "<type>19</type><recorditem><![CDATA[<recordinfo><datalist>{items}</datalist></recordinfo>]]></recorditem>"
        ));
        let card = parse(&xml).unwrap().unwrap();
        assert!(matches!(&card,MessageCard::Forward{entries,omitted:2,..} if entries.len()==100));
    }
    #[test]
    fn ambiguous_broken_and_entity_xml_rejected() {
        for xml in [
            app("<type>5</type><type>57</type>"),
            app("<type>5</type><title><b>nested</b></title>"),
            app("<type>5</type><title>&unknown;</title>"),
            "<!DOCTYPE msg [<!ENTITY x SYSTEM 'file:///private'>]><msg/>".into(),
            "<msg/><msg/>".into(),
            "<msg><appmsg>".into(),
            app(
                "<type>19</type><recorditem><![CDATA[<!DOCTYPE recordinfo><recordinfo/>]]></recorditem>",
            ),
        ] {
            assert!(parse(&xml).is_err())
        }
        assert!(parse(&"x".repeat(MAX_XML + 1)).is_err());
    }
    #[test]
    fn unsupported_card_stays_unsupported() {
        assert!(
            parse(&app("<type>6</type><title>file</title>"))
                .unwrap()
                .is_none()
        )
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    #[test]
    fn additional_shares_expose_only_display_fields_never_execution_or_media_keys() {
        for (number, expected) in [
            (3, "music"),
            (4, "video"),
            (33, "mini_program"),
            (36, "mini_program"),
        ] {
            let xml = format!(
                "<msg><appmsg><type>{number}</type><title>A &amp; B</title><des><![CDATA[<script>literal</script>]]></des><sourcedisplayname>Owned app</sourcedisplayname><url>https://example.test/watch</url><dataurl>https://example.test/token</dataurl><weappinfo><appid>private-app-id</appid><username>private-name</username><pagepath>private-route</pagepath></weappinfo><appattach><aeskey>private-key</aeskey></appattach></appmsg></msg>"
            );
            let parsed = parse(&xml).unwrap().unwrap();
            let value = serde_json::to_value(&parsed).unwrap();
            assert_eq!(value["kind"], "app_share");
            assert_eq!(value["category"], expected);
            assert_eq!(value["title"], "A & B");
            assert_eq!(value["source_name"], "Owned app");
            assert!(!value.to_string().contains("private-"));
            assert!(!value.to_string().contains("example.test/token"));
        }
    }
    #[test]
    fn share_types_and_display_fields_remain_strict() {
        for body in [
            "<type>33</type><type>36</type>",
            "<type>3</type><title>A</title><title>B</title>",
            "<type>4</type><sourcedisplayname>A</sourcedisplayname><sourcedisplayname>B</sourcedisplayname>",
        ] {
            assert!(parse(&format!("<msg><appmsg>{body}</appmsg></msg>")).is_err());
        }
        assert!(
            parse("<msg><appmsg><type>999</type><title>unknown</title></appmsg></msg>")
                .unwrap()
                .is_none()
        );
        let v = serde_json::to_value(
            parse("<msg><appmsg><type>33</type><url>weixin://private</url></appmsg></msg>")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(v["url"].is_null());
    }
}

#[cfg(test)]
mod typed_card_tests {
    use super::*;
    #[test]
    fn contact_and_location_only_return_display_attributes() {
        let contact=parse_for_type(42,r#"<msg username="owned_id" nickname="A &amp; B" bigheadimgurl="https://private.test/avatar" ticket="secret"/>"#).unwrap().unwrap();
        let contact = serde_json::to_value(contact).unwrap();
        assert_eq!(contact["nickname"], "A & B");
        assert_eq!(contact["username"], "owned_id");
        assert!(!contact.to_string().contains("secret"));
        assert!(!contact.to_string().contains("private.test"));
        let location=parse_for_type(48,r#"<msg><location x="31.25" y="121.5" label="Road &amp; street" poiname="Owned place" mapurl="private"/></msg>"#).unwrap().unwrap();
        let location = serde_json::to_value(location).unwrap();
        assert_eq!(location["latitude"], "31.25");
        assert_eq!(location["longitude"], "121.5");
        assert_eq!(location["label"], "Road & street");
        assert!(!location.to_string().contains("private"));
    }
    #[test]
    fn invalid_coordinates_and_ambiguous_fields_rejected() {
        for (x, y) in [
            ("NaN", "0"),
            ("inf", "0"),
            ("91", "0"),
            ("0", "181"),
            ("", "0"),
            ("0", "oops"),
        ] {
            assert!(
                parse_for_type(48, &format!(r#"<msg><location x="{x}" y="{y}"/></msg>"#)).is_err()
            );
        }
        for xml in [
            r#"<msg><location x="0" x="1" y="2"/></msg>"#,
            r#"<msg><location x="0" y="1"/><location x="2" y="3"/></msg>"#,
            r#"<!DOCTYPE msg><msg><location x="0" y="1"/></msg>"#,
        ] {
            assert!(parse_for_type(48, xml).is_err());
        }
        for xml in [
            r#"<msg username="a" username="b"/>"#,
            r#"<msg nickname="No ID"/>"#,
            r#"<msg username="a b"/>"#,
            r#"<msg username="a" nickname="&#xA;"/>"#,
        ] {
            assert!(parse_for_type(42, xml).is_err());
        }
        assert!(parse_for_type(1, "not xml").unwrap().is_none());
        assert!(parse_for_type(49, r#"<msg username="a"/>"#).is_err());
    }
}
