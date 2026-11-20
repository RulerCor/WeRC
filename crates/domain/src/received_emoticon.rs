//! A local cache selector derived only from authenticated type47 metadata.
use crate::{AppError, AppResult, receive::ReceivedText};
use quick_xml::{Reader, XmlVersion, events::Event};
use zeroize::Zeroizing;
pub struct ReceivedEmoticonDescriptor {
    pub converted_cache: bool,
    pub md5: Zeroizing<String>,
    pub declared_size: Option<usize>,
    pub width: Option<u16>,
    pub height: Option<u16>,
}
fn invalid() -> AppError {
    AppError::new(
        "RECEIVED_EMOTICON_INVALID",
        "此表情没有受支持的本地缓存标识",
    )
}
pub fn descriptor(message: &ReceivedText) -> AppResult<ReceivedEmoticonDescriptor> {
    if message.message_type == 49 {
        return app_descriptor(message);
    }
    if message.message_type != 47 || !message.valid_content() {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(message.metadata.as_deref().ok_or_else(invalid)?);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<String>::new();
    let mut root = false;
    let mut seen = false;
    let mut md5 = None;
    let mut declared_size = None;
    let mut width = None;
    let mut height = None;
    let mut events = 0;
    loop {
        events += 1;
        if events > 2048 {
            return Err(invalid());
        }
        match reader.read_event().map_err(|_| invalid())? {
            Event::Start(e) => {
                let name = e.name().as_ref().to_owned();
                if stack.len() >= 16 || name.len() > 64 {
                    return Err(invalid());
                }
                if stack.is_empty() {
                    if root || name != "msg" {
                        return Err(invalid());
                    }
                    root = true;
                }
                let emoji = stack.as_slice() == ["msg"] && name == "emoji";
                if emoji && seen {
                    return Err(invalid());
                }
                if emoji {
                    seen = true;
                }
                for (index, a) in e.attributes().enumerate() {
                    if index >= 64 {
                        return Err(invalid());
                    }
                    let a = a.map_err(|_| invalid())?;
                    let value = Zeroizing::new(
                        a.normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|_| invalid())?
                            .into_owned(),
                    );
                    if emoji {
                        match a.key.as_ref() {
                            "md5" => md5 = Some(value),
                            "len" => {
                                declared_size = Some(value.parse::<usize>().map_err(|_| invalid())?)
                            }
                            "width" => width = Some(value.parse::<u16>().map_err(|_| invalid())?),
                            "height" => height = Some(value.parse::<u16>().map_err(|_| invalid())?),
                            _ => {}
                        }
                    }
                }
                stack.push(name);
            }
            Event::End(e) => {
                if stack.pop().as_deref() != Some(e.name().as_ref()) {
                    return Err(invalid());
                }
            }
            Event::Text(t) => {
                if stack.is_empty() && !t.xml10_content().trim().is_empty() {
                    return Err(invalid());
                }
            }
            Event::Comment(_) => {}
            Event::CData(_) if !stack.is_empty() => {}
            Event::Decl(_) if !root => {}
            Event::Eof => break,
            _ => return Err(invalid()),
        }
    }
    let mut md5 = md5.ok_or_else(invalid)?;
    if !root
        || !seen
        || !stack.is_empty()
        || md5.len() != 32
        || !md5.bytes().all(|b| b.is_ascii_hexdigit())
        || declared_size.is_some_and(|n| n == 0 || n > 1024 * 1024)
        || [width, height]
            .into_iter()
            .flatten()
            .any(|n| n == 0 || n > 1024)
    {
        return Err(invalid());
    }
    md5.make_ascii_lowercase();
    Ok(ReceivedEmoticonDescriptor {
        converted_cache: false,
        md5,
        declared_size,
        width,
        height,
    })
}
impl ReceivedEmoticonDescriptor {
    pub fn validate_bytes(&self, bytes: &[u8]) -> AppResult<crate::emoticon::EmoticonInfo> {
        use md5::{Digest, Md5};
        if self.converted_cache && bytes.starts_with(b"wxgf") {
            return Ok(crate::wxgf::parse(bytes)?.info);
        }
        let info = crate::emoticon::inspect_gif("received-emoticon.gif", bytes)?;
        if !hex::encode(Md5::digest(bytes)).eq_ignore_ascii_case(&self.md5)
            || self.declared_size.is_some_and(|n| n != bytes.len())
            || self.width.is_some_and(|n| n != info.width)
            || self.height.is_some_and(|n| n != info.height)
        {
            return Err(invalid());
        }
        Ok(info)
    }
}

// Appmsg type8's totallen describes the original asset, not its cached wxgf
// representation. It must never be used as the cache byte length or digest.
fn app_descriptor(message: &ReceivedText) -> AppResult<ReceivedEmoticonDescriptor> {
    if !message.valid_content() {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(message.metadata.as_deref().ok_or_else(invalid)?);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<String>::new();
    let (mut root, mut app, mut attach) = (false, false, false);
    let (mut kind, mut md5) = (None, None);
    let mut field: Option<(bool, Zeroizing<String>)> = None;
    for _ in 0..2048 {
        match reader.read_event().map_err(|_| invalid())? {
            Event::Start(e) => {
                let name = e.name().as_ref().to_owned();
                if stack.len() >= 16 || name.len() > 64 || field.is_some() {
                    return Err(invalid());
                }
                if stack.is_empty() {
                    if root || name != "msg" {
                        return Err(invalid());
                    }
                    root = true;
                }
                if stack.as_slice() == ["msg"] && name == "appmsg" {
                    if app {
                        return Err(invalid());
                    }
                    app = true;
                }
                if stack.as_slice() == ["msg", "appmsg"] && name == "appattach" {
                    if attach {
                        return Err(invalid());
                    }
                    attach = true;
                }
                let is_type = stack.as_slice() == ["msg", "appmsg"] && name == "type";
                let is_hash =
                    stack.as_slice() == ["msg", "appmsg", "appattach"] && name == "emoticonmd5";
                for (i, a) in e.attributes().enumerate() {
                    if i >= 64 || a.is_err() {
                        return Err(invalid());
                    }
                }
                if is_type || is_hash {
                    if (is_type && kind.is_some()) || (is_hash && md5.is_some()) {
                        return Err(invalid());
                    }
                    field = Some((is_hash, Zeroizing::new(String::new())));
                }
                stack.push(name);
            }
            Event::Text(t) => {
                let value = t.xml10_content();
                if let Some((_, s)) = &mut field {
                    if s.len() + value.len() > 128 {
                        return Err(invalid());
                    }
                    s.push_str(&value);
                } else if stack.is_empty() && !value.trim().is_empty() {
                    return Err(invalid());
                }
            }
            Event::CData(t) => {
                if let Some((_, s)) = &mut field {
                    let value = t.xml10_content();
                    if s.len() + value.len() > 128 {
                        return Err(invalid());
                    }
                    s.push_str(&value);
                } else if stack.is_empty() {
                    return Err(invalid());
                }
            }
            Event::End(e) => {
                if stack.pop().as_deref() != Some(e.name().as_ref()) {
                    return Err(invalid());
                }
                if let Some((hash, text)) = field.take() {
                    if hash {
                        md5 = Some(text);
                    } else {
                        kind = Some(text);
                    }
                }
            }
            Event::Comment(_) => {}
            Event::Decl(_) if !root => {}
            Event::GeneralRef(_) if field.is_none() && !stack.is_empty() => {}
            Event::Eof => {
                if !root
                    || !app
                    || !attach
                    || !stack.is_empty()
                    || kind.as_deref().map(|v| v.trim()) != Some("8")
                {
                    return Err(invalid());
                }
                let mut md5 = md5.ok_or_else(invalid)?;
                if md5.len() != 32 || !md5.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid());
                }
                md5.make_ascii_lowercase();
                return Ok(ReceivedEmoticonDescriptor {
                    converted_cache: true,
                    md5,
                    declared_size: None,
                    width: None,
                    height: None,
                });
            }
            _ => return Err(invalid()),
        }
    }
    Err(invalid())
}
impl ReceivedEmoticonDescriptor {
    pub fn stored_name(&self, bytes: &[u8]) -> &'static str {
        if self.converted_cache && bytes.starts_with(b"wxgf") {
            "received-emoticon.wxgf"
        } else {
            "received-emoticon.gif"
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn row(xml: String) -> ReceivedText {
        ReceivedText {
            sequence: 1,
            native_message_id: "1".into(),
            text: String::new(),
            message_type: 47,
            metadata: Some(xml),
        }
    }
    fn xml() -> String {
        format!(
            "<msg><emoji md5=\"{}\" len=\"11181\" width=\"320\" height=\"180\"/></msg>",
            "AB".repeat(16)
        )
    }
    #[test]
    fn appmsg8_uses_original_selector_not_original_size_as_cache_size() {
        let xml = format!(
            "<msg><appmsg><title>owned</title><type><![CDATA[8]]></type><appattach><totallen>2641358</totallen><fileext>hevc</fileext><emoticonmd5>{}</emoticonmd5></appattach></appmsg></msg>",
            "ab".repeat(16)
        );
        let mut message = row(xml.clone());
        message.message_type = 49;
        let d = descriptor(&message).unwrap();
        assert!(d.converted_cache);
        assert!(d.declared_size.is_none());
        let cache = include_bytes!("../../../tests/fixtures/owned-received-emoticon.wxgf");
        assert_eq!(d.validate_bytes(cache).unwrap().frames, 3);
        assert_eq!(d.stored_name(cache), "received-emoticon.wxgf");
        message.message_type = 47;
        assert!(descriptor(&message).is_err());
        for bad in [
            xml.replace("[8]", "[6]"),
            xml.replace("</appmsg>", "<type>8</type></appmsg>"),
            xml.replace("<appattach>", "<x><appattach>")
                .replace("</appattach>", "</appattach></x>"),
            xml.replace(&"ab".repeat(16), "../secret"),
            xml.replace("<![CDATA[8]]>", "<x>8</x>"),
            format!("<!DOCTYPE msg>{xml}"),
            format!("{xml}{xml}"),
        ] {
            let mut m = row(bad);
            m.message_type = 49;
            assert!(descriptor(&m).is_err());
        }
        let original = descriptor(&row(format!(
            "<msg><emoji md5=\"{}\"/></msg>",
            "ab".repeat(16)
        )))
        .unwrap();
        assert!(original.validate_bytes(cache).is_err());
    }
    #[test]
    fn accepts_bounded_selector_and_normalizes_hash() {
        let d = descriptor(&row(xml())).unwrap();
        assert_eq!(&*d.md5, &"ab".repeat(16));
        assert_eq!(d.declared_size, Some(11181));
        assert_eq!((d.width, d.height), (Some(320), Some(180)));
    }
    #[test]
    fn rejects_nested_ambiguous_external_and_oversized_metadata() {
        let good = xml();
        for bad in [
            good.replace("<msg>", "<msg><x>")
                .replace("</msg>", "</x></msg>"),
            format!("{good}{good}"),
            good.replace("</msg>", "<emoji/></msg>"),
            good.replace("11181", "1048577"),
            good.replace("320", "1025"),
            good.replace("md5=", "md5=\"duplicate\" md5="),
            good.replace("</msg>", ""),
            format!("<!DOCTYPE msg [<!ENTITY x SYSTEM 'file:///owned'>]>{good}"),
            good.replace(&"AB".repeat(16), "../not-a-hash"),
        ] {
            assert!(descriptor(&row(bad)).is_err());
        }
        let mut wrong = row(good);
        wrong.message_type = 49;
        assert!(descriptor(&wrong).is_err());
    }
    #[test]
    fn hash_only_descriptor_has_no_guessed_size() {
        let d = descriptor(&row(format!(
            "<msg><emoji md5=\"{}\"/></msg>",
            "ab".repeat(16)
        )))
        .unwrap();
        assert!(d.declared_size.is_none() && d.width.is_none() && d.height.is_none());
    }
}
