//! Strict, bounded descriptors from authenticated received observations.
//! CDN credentials deliberately implement neither Debug nor Serialize.
use crate::{AppError, AppResult, receive::ReceivedText};
use quick_xml::{Reader, XmlVersion, events::Event};
use zeroize::Zeroizing;

pub const RECEIVED_MEDIA_MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const RECEIVED_VIDEO_MAX_BYTES: u64 = 16 * 1024 * 1024;
pub struct ReceivedImageBinding<'a> {
    pub observation_id: &'a str,
    pub connection_revision: i64,
    pub proof: &'a str,
}
pub struct ReceivedImageLink {
    pub asset_id: String,
    pub proof: String,
}
pub struct ReceivedImageDescriptor {
    pub file_key: Zeroizing<String>,
    pub aes_key: Zeroizing<String>,
    /// Admission hint; native output may be transformed. It has its own cap.
    pub declared_size: u64,
}
fn invalid() -> AppError {
    AppError::new("RECEIVED_MEDIA_INVALID", "媒体元数据不符合已支持的格式，请勿原样重试")
}
/// Parse only msg/img's medium image. Never chooses a URL or filesystem path.
pub fn image_descriptor(message: &ReceivedText) -> AppResult<ReceivedImageDescriptor> {
    image_descriptor_in_conversation(message, false)
}
/// A verified group observation may contain the native member envelope before XML.
pub fn image_descriptor_in_conversation(
    message: &ReceivedText,
    is_verified_group: bool,
) -> AppResult<ReceivedImageDescriptor> {
    attribute_descriptor(
        message, 3, "img", "cdnmidimgurl", RECEIVED_MEDIA_MAX_BYTES, is_verified_group,
    )
}
pub struct ReceivedVideoDescriptor {
    pub media: ReceivedImageDescriptor,
}
/// Only the ordinary received video object; no raw-video, thumbnail or URL fallback.
pub fn video_descriptor(message: &ReceivedText) -> AppResult<ReceivedVideoDescriptor> {
    Ok(ReceivedVideoDescriptor {
        media: attribute_descriptor(
            message,
            43,
            "videomsg",
            "cdnvideourl",
            RECEIVED_VIDEO_MAX_BYTES,
            false,
        )?,
    })
}
fn attribute_descriptor(
    message: &ReceivedText,
    message_type: u32,
    element: &str,
    key_attribute: &str,
    maximum: u64,
    is_verified_group: bool,
) -> AppResult<ReceivedImageDescriptor> {
    if message.message_type != message_type || !message.valid_content() {
        return Err(invalid());
    }
    let metadata = message.metadata.as_deref().ok_or_else(invalid)?;
    let xml = if is_verified_group && !metadata.starts_with('<') {
        let (member, xml) = metadata.split_once(":\n").ok_or_else(invalid)?;
        if member.is_empty()
            || member.len() >= 128
            || !member.bytes().all(|b|
                b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@'))
            || !xml.starts_with('<')
        {
            return Err(invalid());
        }
        xml
    } else {
        metadata
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::new();
    let mut root_seen = false;
    let mut image_seen = false;
    let mut file_key = None;
    let mut aes_key = None;
    let mut size = None;
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
                    if root_seen || name != "msg" {
                        return Err(invalid());
                    }
                    root_seen = true;
                }
                let image = stack.as_slice() == ["msg"] && name == element;
                if image && image_seen {
                    return Err(invalid());
                }
                if image {
                    image_seen = true;
                }
                let mut count = 0;
                for attr in e.attributes() {
                    count += 1;
                    if count > 64 {
                        return Err(invalid());
                    }
                    let attr = attr.map_err(|_| invalid())?;
                    let value = Zeroizing::new(
                        attr.normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|_| invalid())?
                            .into_owned(),
                    );
                    if image {
                        match attr.key.as_ref() {
                            key if key == key_attribute => file_key = Some(value),
                            "aeskey" => aes_key = Some(value),
                            "length" => size = Some(value.parse::<u64>().map_err(|_| invalid())?),
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
                let value = t.xml10_content();
                if stack.is_empty() && !value.trim().is_empty() {
                    return Err(invalid());
                }
            }
            Event::CData(_) if !stack.is_empty() => {}
            Event::Comment(_) => {}
            Event::Decl(_) if !root_seen => {}
            Event::Eof => break,
            // DTD, PI and external/custom entities never enter a descriptor.
            _ => return Err(invalid()),
        }
    }
    let key = file_key.ok_or_else(invalid)?;
    let aes = aes_key.ok_or_else(invalid)?;
    let expected = size.ok_or_else(invalid)?;
    if !root_seen
        || !stack.is_empty()
        || !(1..=maximum).contains(&expected)
        || key.is_empty()
        || key.len() > 768
        || key.len() % 2 != 0
        || !key.bytes().all(|b| b.is_ascii_hexdigit())
        || aes.len() != 32
        || !aes.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    Ok(ReceivedImageDescriptor {
        file_key: key,
        aes_key: aes,
        declared_size: expected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(xml: String) -> ReceivedText {
        ReceivedText {
            sequence: 1,
            native_message_id: "1".into(),
            text: String::new(),
            message_type: 3,
            metadata: Some(xml),
        }
    }
    fn xml(extra: &str) -> String {
        format!(
            "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabbccdd\" length=\"100\" {extra}/></msg>",
            "ab".repeat(16)
        )
    }
    #[test]
    fn accepts_bounded_image_without_exposing_credentials() {
        let d = image_descriptor(&message(xml(""))).unwrap();
        assert_eq!(d.declared_size, 100);
        assert_eq!(d.file_key.len(), 8);
    }
    #[test]
    fn synthetic_group_observations_preserve_the_native_envelope_and_field_shapes() {
        // Synthetic member and CDN values; lengths and XML envelope reflect
        // the three observed group-image shapes, without message content or keys.
        for (sample, size) in [
            (include_str!("../../../tests/fixtures/received-image-group-1823-synthetic.xml"), 37934),
            (include_str!("../../../tests/fixtures/received-image-group-1836-synthetic.xml"), 41593),
            (include_str!("../../../tests/fixtures/received-image-group-1895-synthetic.xml"), 40007),
        ] {
            let m = message(sample.to_owned());
            assert!(image_descriptor(&m).is_err());
            let d = image_descriptor_in_conversation(&m, true).unwrap();
            assert_eq!(d.file_key.len(), 194);
            assert_eq!(d.aes_key.len(), 32);
            assert_eq!(d.declared_size, size);
            for bad in [
                sample.replace("wxid_synthetic00000", "bad/id"),
                sample.replace(":\n", ": "),
                sample.replace("cdnmidimgurl=", "missingkey="),
                sample.replace(&"d4".repeat(97), "https://example.invalid"),
                sample.replace(&"a1".repeat(16), "short"),
                sample.replace(&format!("length=\"{size}\""), "length=\"8388609\""),
            ] {
                assert!(image_descriptor_in_conversation(&message(bad), true).is_err());
            }
        }
    }
    #[test]
    fn video_uses_its_own_type_and_exact_descriptor() {
        let xml = xml("")
            .replace("<img ", "<videomsg ")
            .replace("cdnmidimgurl", "cdnvideourl");
        let mut m = message(xml.clone());
        assert!(video_descriptor(&m).is_err());
        m.message_type = 43;
        assert_eq!(video_descriptor(&m).unwrap().media.declared_size, 100);
        assert!(image_descriptor(&m).is_err());
        for bad in [
            xml.replace("cdnvideourl", "cdnrawvideourl"),
            xml.replace("cdnvideourl", "cdnthumburl"),
            xml.replace("</msg>", "<videomsg/></msg>"),
            xml.replace("<msg>", "<msg><x>")
                .replace("</msg>", "</x></msg>"),
            xml.replace("100", "16777217"),
            xml.replace("aabbccdd", "https://example.invalid"),
        ] {
            m.metadata = Some(bad);
            assert!(video_descriptor(&m).is_err());
        }
    }
    #[test]
    fn rejects_ambiguous_nested_or_malformed_images() {
        let base = xml("");
        for bad in [
            base.replace("<msg>", "<msg><other>")
                .replace("</msg>", "</other></msg>"),
            base.replace("</msg>", "<img/></msg>"),
            format!("{base}{base}"),
            xml("aeskey=\"duplicate\""),
            base.replace("</msg>", ""),
            base.replace("aabbccdd", "https://example.invalid/"),
            base.replace("100", "8388609"),
            base.replace("100", "0"),
            format!("<!DOCTYPE msg [<!ENTITY key SYSTEM 'file:///test'>]>{base}"),
            base.replace("aabbccdd", "&key;"),
        ] {
            assert!(image_descriptor(&message(bad)).is_err());
        }
    }
}

/// Ordinary attachment only (appmsg/type=6). No URL or path is accepted.
pub struct ReceivedFileDescriptor {
    pub media: ReceivedImageDescriptor,
    pub file_name: Zeroizing<String>,
}
pub fn file_descriptor(message: &ReceivedText) -> AppResult<ReceivedFileDescriptor> {
    if message.message_type != 49 || !message.valid_content() {
        return Err(invalid());
    }
    let mut reader = Reader::from_str(message.metadata.as_deref().ok_or_else(invalid)?);
    reader.config_mut().expand_empty_elements = true;
    let paths = [
        "msg/appmsg/type",
        "msg/appmsg/title",
        "msg/appmsg/appattach/totallen",
        "msg/appmsg/appattach/cdnattachurl",
        "msg/appmsg/appattach/aeskey",
    ];
    let mut values: [Option<Zeroizing<String>>; 5] = Default::default();
    let mut stack = Vec::<String>::new();
    let (mut roots, mut apps, mut attaches, mut events) = (0, 0, 0, 0);
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
                if paths.contains(&stack.join("/").as_str()) {
                    return Err(invalid());
                }
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || name != "msg" {
                        return Err(invalid());
                    }
                }
                for (i, a) in e.attributes().enumerate() {
                    if i >= 64 {
                        return Err(invalid());
                    }
                    a.map_err(|_| invalid())?;
                }
                stack.push(name);
                let path = stack.join("/");
                if path == "msg/appmsg" {
                    apps += 1;
                    if apps > 1 {
                        return Err(invalid());
                    }
                }
                if path == "msg/appmsg/appattach" {
                    attaches += 1;
                    if attaches > 1 {
                        return Err(invalid());
                    }
                }
                if let Some(i) = paths.iter().position(|p| *p == path) {
                    if values[i].is_some() {
                        return Err(invalid());
                    }
                    values[i] = Some(Zeroizing::new(String::new()));
                }
            }
            Event::End(e) => {
                if stack.pop().as_deref() != Some(e.name().as_ref()) {
                    return Err(invalid());
                }
            }
            Event::Text(t) => {
                let text = t.xml10_content();
                if stack.is_empty() && !text.trim().is_empty() {
                    return Err(invalid());
                }
                if let Some(i) = paths.iter().position(|p| *p == stack.join("/")) {
                    values[i].as_mut().ok_or_else(invalid)?.push_str(&text);
                }
            }
            Event::CData(t) if !stack.is_empty() => {
                if let Some(i) = paths.iter().position(|p| *p == stack.join("/")) {
                    values[i]
                        .as_mut()
                        .ok_or_else(invalid)?
                        .push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) if !stack.is_empty() => {
                let decoded = r.xml10_content();
                let value = match decoded.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => return Err(invalid()),
                };
                if let Some(i) = paths.iter().position(|p| *p == stack.join("/")) {
                    values[i].as_mut().ok_or_else(invalid)?.push_str(value);
                }
            }
            Event::Comment(_) => {}
            Event::Decl(_) if roots == 0 => {}
            Event::Eof => break,
            _ => return Err(invalid()),
        }
    }
    let [kind, name, size, key, aes] = values;
    let name = name.ok_or_else(invalid)?;
    let key = key.ok_or_else(invalid)?;
    let aes = aes.ok_or_else(invalid)?;
    let size = size
        .ok_or_else(invalid)?
        .parse::<u64>()
        .map_err(|_| invalid())?;
    if !stack.is_empty()
        || kind.as_deref().map(|s| s.as_str()) != Some("6")
        || !(1..=RECEIVED_MEDIA_MAX_BYTES).contains(&size)
        || key.is_empty()
        || key.len() > 768
        || key.len() % 2 != 0
        || !key.bytes().all(|b| b.is_ascii_hexdigit())
        || aes.len() != 32
        || !aes.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    crate::file::validate_file_name(&name).map_err(|_| invalid())?;
    Ok(ReceivedFileDescriptor {
        file_name: name,
        media: ReceivedImageDescriptor {
            file_key: key,
            aes_key: aes,
            declared_size: size,
        },
    })
}

#[cfg(test)]
mod file_tests {
    use super::*;
    fn message(xml: String) -> ReceivedText {
        ReceivedText {
            sequence: 1,
            native_message_id: "1".into(),
            text: String::new(),
            message_type: 49,
            metadata: Some(xml),
        }
    }
    fn xml() -> String {
        format!(
            "<msg><appmsg><title><![CDATA[自动化测试.txt]]></title><type>6</type><appattach><totallen>73</totallen><cdnattachurl>aabbccdd</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
            "ab".repeat(16)
        )
    }
    #[test]
    fn ordinary_file_uses_exact_fields() {
        let d = file_descriptor(&message(xml())).unwrap();
        assert_eq!(d.media.declared_size, 73);
        assert_eq!(&*d.file_name, "自动化测试.txt");
        assert!(
            file_descriptor(&message(
                xml().replace("<![CDATA[自动化测试.txt]]>", "a&amp;b.txt")
            ))
            .is_ok()
        );
    }
    #[test]
    fn wrong_subtype_duplicate_nested_path_and_oversize_are_rejected() {
        for x in [
            xml().replace("<type>6", "<type>8"),
            xml().replace("</type>", "</type><type>6</type>"),
            xml().replace("<type>6", "<type><x>6</x>"),
            xml().replace("73", "8388609"),
            xml().replace("73", "0"),
            xml().replace("自动化测试.txt", "../x"),
            xml().replace("aabbccdd", "https://example.invalid"),
            format!("{}{}", xml(), xml()),
            format!("<!DOCTYPE msg>{}", xml()),
            xml().replace("</msg>", ""),
            xml().replace("</appmsg>", "<appattach/></appmsg>"),
        ] {
            assert!(file_descriptor(&message(x)).is_err());
        }
    }
}

#[derive(serde::Serialize)]
pub struct ReceivedFileInfo {
    pub file_name: String,
    pub size: u64,
}
