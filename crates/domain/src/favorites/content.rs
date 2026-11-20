use super::*;
use quick_xml::{Reader, events::Event};
use zeroize::Zeroizing;
const INVALID: &str = "FAVORITE_CONTENT_INVALID";

#[derive(Clone, Serialize, Deserialize)]
pub struct FavoriteBlock {
    pub slot: Vec<usize>,
    pub kind: u32,
    pub title: String,
    pub text: String,
    pub format: String,
    pub size: Option<u64>,
    pub duration: Option<u64>,
    pub source_name: String,
    pub source_time: String,
    pub media_available: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FavoriteContent {
    pub title: String,
    pub text: String,
    pub tags: Vec<String>,
    pub url: Option<String>,
    pub author: String,
    pub blocks: Vec<FavoriteBlock>,
}
struct Node {
    name: String,
    text: Zeroizing<String>,
    kind: Option<u32>,
    children: Vec<Node>,
}
impl Node {
    fn child(&self, name: &str) -> Result<Option<&Self>, &'static str> {
        let mut it = self.children.iter().filter(|v| v.name == name);
        let v = it.next();
        if it.next().is_some() {
            Err(INVALID)
        } else {
            Ok(v)
        }
    }
    fn value(&self, name: &str) -> Result<&str, &'static str> {
        match self.child(name)? {
            Some(v) if v.children.is_empty() => Ok(&v.text),
            None => Ok(""),
            _ => Err(INVALID),
        }
    }
}
fn document(xml: &str) -> Result<Node, &'static str> {
    if xml.len() > MAX_XML || xml.contains('\0') {
        return Err(INVALID);
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut count = 0;
    let mut declaration = false;
    loop {
        match reader.read_event().map_err(|_| INVALID)? {
            Event::Start(e) => {
                count += 1;
                let name = e.name().as_ref().to_owned();
                if count > 8192
                    || stack.len() >= 32
                    || name.len() > 64
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || (stack.is_empty() && root.is_some())
                {
                    return Err(INVALID);
                }
                let mut kind = None;
                for (i, a) in e.attributes().enumerate() {
                    if i >= 64 {
                        return Err(INVALID);
                    }
                    let a = a.map_err(|_| INVALID)?;
                    if matches!(
                        (name.as_str(), a.key.as_ref()),
                        ("favitem", "type") | ("dataitem", "datatype")
                    ) {
                        if kind.is_some() {
                            return Err(INVALID);
                        }
                        kind = Some(
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map_err(|_| INVALID)?
                                .parse::<u32>()
                                .map_err(|_| INVALID)?,
                        );
                    }
                }
                stack.push(Node {
                    name,
                    text: Zeroizing::new(String::new()),
                    kind,
                    children: Vec::new(),
                });
            }
            Event::Text(e) => append(&mut stack, &e.xml10_content())?,
            Event::CData(e) if !stack.is_empty() => append(&mut stack, &e.xml10_content())?,
            Event::GeneralRef(e) if !stack.is_empty() => {
                let name = e.xml10_content();
                let value = match name.as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => e.resolve_char_ref().map_err(|_| INVALID)?.ok_or(INVALID)?,
                };
                append(&mut stack, &value.to_string())?;
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
            Event::Decl(_) if !declaration && root.is_none() && stack.is_empty() => {
                declaration = true
            }
            Event::Comment(_) => {}
            Event::Eof => break,
            _ => return Err(INVALID),
        }
    }
    if !stack.is_empty() {
        return Err(INVALID);
    }
    root.ok_or(INVALID)
}
fn append(stack: &mut [Node], text: &str) -> Result<(), &'static str> {
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(INVALID);
    }
    if let Some(n) = stack.last_mut() {
        // Credentials/paths retained only for an explicit media read, never projected.
        if matches!(
            n.name.as_str(),
            "type"
                | "title"
                | "desc"
                | "datatitle"
                | "datadesc"
                | "datafmt"
                | "fullsize"
                | "duration"
                | "datasrcname"
                | "datasrctime"
                | "tag"
                | "link"
                | "pagetitle"
                | "pagedesc"
                | "nickname"
                | "webUrl"
                | "recordxml"
                | "cdn_dataurl"
                | "cdn_datakey"
                | "fullmd5"
                | "sourcedatapath"
        ) {
            n.text.push_str(text)
        }
    } else if !text.trim().is_empty() {
        return Err(INVALID);
    }
    Ok(())
}
fn safe_url(raw: &str) -> Option<String> {
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
fn root_type(root: &Node, expected: u32) -> bool {
    let Ok(value) = root.value("type") else {
        return false;
    };
    let element = if value.trim().is_empty() {
        None
    } else {
        match value.trim().parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => return false,
        }
    };
    root.kind.or(element) == Some(expected)
        && !matches!((root.kind,element),(Some(a),Some(b)) if a!=b)
}
pub fn parse_content(xml: &str, expected: u32) -> Result<FavoriteContent, &'static str> {
    let root = document(xml)?;
    if root.name != "favitem" || !root_type(&root, expected) {
        return Err(INVALID);
    }
    let mut result = FavoriteContent {
        title: root.value("title")?.into(),
        text: root.value("desc")?.into(),
        tags: vec![],
        url: None,
        author: String::new(),
        blocks: vec![],
    };
    if let Some(tags) = root.child("taglist")? {
        for tag in &tags.children {
            if tag.name == "tag" && tag.children.is_empty() {
                if tag.text.len() > 256 || result.tags.len() >= 128 {
                    return Err(INVALID);
                }
                result.tags.push(tag.text.to_string());
            }
        }
    }
    if let Some(source) = root.child("source")? {
        result.url = safe_url(source.value("link")?);
    }
    if let Some(web) = root.child("weburlitem")? {
        if result.title.is_empty() {
            result.title = web.value("pagetitle")?.into()
        }
        if result.text.is_empty() {
            result.text = web.value("pagedesc")?.into()
        }
    }
    if let Some(finder) = root.child("finderFeed")? {
        result.author = finder.value("nickname")?.into();
        result.url = safe_url(finder.value("webUrl")?).or(result.url);
        if result.text.is_empty() {
            result.text = finder.value("desc")?.into()
        }
    }
    collect(&root, &[], 0, &mut result.blocks)?;
    if result.title.len() > 16384 || result.text.len() > 65536 {
        return Err(INVALID);
    }
    Ok(result)
}
fn collect(
    root: &Node,
    parent: &[usize],
    depth: usize,
    out: &mut Vec<FavoriteBlock>,
) -> Result<(), &'static str> {
    let Some(list) = root.child("datalist")? else {
        return Ok(());
    };
    for (index, item) in list
        .children
        .iter()
        .filter(|v| v.name == "dataitem")
        .enumerate()
    {
        if out.len() >= 256 {
            return Err(INVALID);
        }
        let mut slot = parent.to_vec();
        slot.push(index);
        let kind = item.kind.unwrap_or(0);
        let media_available =
            !item.value("cdn_dataurl")?.is_empty() && !item.value("cdn_datakey")?.is_empty();
        out.push(FavoriteBlock {
            slot: slot.clone(),
            kind,
            title: item.value("datatitle")?.into(),
            text: item.value("datadesc")?.into(),
            format: item.value("datafmt")?.into(),
            size: item.value("fullsize")?.parse().ok(),
            duration: item.value("duration")?.parse().ok(),
            source_name: item.value("datasrcname")?.into(),
            source_time: item.value("datasrctime")?.into(),
            media_available,
        });
        if depth < 3 {
            if let Some(nested) = item.child("recordxml")? {
                if !nested.text.is_empty() {
                    let node = document(&nested.text)?;
                    collect(&node, &slot, depth + 1, out)?
                } else {
                    collect(nested, &slot, depth + 1, out)?
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_favorite_projection_omits_transport_secrets_and_preserves_unknown_kinds() {
        let xml = "<favitem type=\"8\"><title>自动化测试 &amp; 文件</title><taglist><tag>测试</tag></taglist><datalist><dataitem datatype=\"8\"><datatitle>x.txt</datatitle><cdn_datakey>private-key</cdn_datakey><cdn_dataurl>private-url</cdn_dataurl><sourcedatapath>C:/private</sourcedatapath><fullsize>73</fullsize></dataitem></datalist></favitem>";
        let p = parse_content(xml, 8).unwrap();
        assert_eq!(p.title, "自动化测试 & 文件");
        assert!(p.blocks[0].media_available);
        assert_eq!(p.blocks[0].size, Some(73));
        let public = serde_json::to_string(&p).unwrap();
        assert!(!public.contains("private"));
        assert!(parse_content(xml, 1).is_err());
        assert!(parse_content("<!DOCTYPE x><favitem type=\"1\"/>", 1).is_err());
        assert!(
            parse_content("<favitem type=\"999\"><desc>未知类型</desc></favitem>", 999).is_ok()
        );
    }
}

// Internal native CDN request; never serialize the credentials.
pub struct FavoriteMedia {
    pub kind: u32,
    pub file_key: Zeroizing<String>,
    pub aes_key: Zeroizing<String>,
    pub size: u64,
    pub name: String,
}
pub fn favorite_media(
    xml: &str,
    expected: u32,
    slot: &[usize],
) -> Result<FavoriteMedia, &'static str> {
    if slot.is_empty() || slot.len() > 4 {
        return Err(INVALID);
    }
    let root = document(xml)?;
    if root.name != "favitem" || !root_type(&root, expected) {
        return Err(INVALID);
    }
    select_media(&root, slot)
}
fn select_media(root: &Node, slot: &[usize]) -> Result<FavoriteMedia, &'static str> {
    let item = root
        .child("datalist")?
        .ok_or(INVALID)?
        .children
        .iter()
        .filter(|v| v.name == "dataitem")
        .nth(slot[0])
        .ok_or(INVALID)?;
    if slot.len() > 1 {
        let nested = item.child("recordxml")?.ok_or(INVALID)?;
        return if nested.text.is_empty() {
            select_media(nested, &slot[1..])
        } else {
            select_media(&document(&nested.text)?, &slot[1..])
        };
    }
    let kind = item.kind.ok_or(INVALID)?;
    if !matches!(kind, 2 | 3 | 4 | 8) {
        return Err("FAVORITE_MEDIA_UNSUPPORTED");
    }
    let key = item.value("cdn_dataurl")?;
    let aes = item.value("cdn_datakey")?;
    let size = item
        .value("fullsize")?
        .parse::<u64>()
        .map_err(|_| INVALID)?;
    let limit = if kind == 4 {
        16 * 1024 * 1024
    } else {
        8 * 1024 * 1024
    };
    if key.is_empty()
        || key.len() > 768
        || key.len() % 2 != 0
        || !key.bytes().all(|v| v.is_ascii_hexdigit())
        || aes.len() != 32
        || !aes.bytes().all(|v| v.is_ascii_hexdigit())
        || size == 0
        || size > limit
    {
        return Err("FAVORITE_MEDIA_UNSUPPORTED");
    }
    Ok(FavoriteMedia {
        kind,
        file_key: Zeroizing::new(key.into()),
        aes_key: Zeroizing::new(aes.into()),
        size,
        name: item.value("datatitle")?.into(),
    })
}

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn link_placeholders_and_legacy_type_element_preserve_display() {
        let xml = "<favitem type=\"5\"><weburlitem><pagetitle>自动化测试旧链接</pagetitle></weburlitem><datalist><dataitem><statextstr>not-display</statextstr></dataitem></datalist></favitem>";
        let c = parse_content(xml, 5).unwrap();
        assert_eq!(c.blocks[0].kind, 0);
        assert!(!c.blocks[0].media_available);
        assert_eq!(c.title, "自动化测试旧链接");
        assert!(parse_content("<favitem><type>5</type><datalist/></favitem>", 5).is_ok());
        assert!(parse_content("<favitem type=\"5\"><type>8</type></favitem>", 5).is_err());
        for url in [
            "https://",
            "https://u@host",
            "https://host\\x",
            "https://host/ x",
        ] {
            assert!(safe_url(url).is_none());
        }
    }
}
