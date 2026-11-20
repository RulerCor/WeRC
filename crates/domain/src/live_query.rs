//! Extract only the direct appmsg/type discriminator, never transport fields.
use quick_xml::{Reader, events::Event};

pub(super) fn app_type(raw: &str, sender: Option<&str>) -> Option<u32> {
    if raw.len() > 16384 || raw.contains('\0') {
        return None;
    }
    let xml = sender
        .and_then(|s| raw.strip_prefix(s))
        .and_then(|s| s.strip_prefix(":\n"))
        .filter(|s| s.starts_with('<'))
        .unwrap_or(raw);
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut path = Vec::<String>::new();
    let mut nodes = 0;
    let mut roots = 0;
    let mut apps = 0;
    let mut types = 0;
    let mut value = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                nodes += 1;
                if nodes > 4096 || path.len() >= 32 {
                    return None;
                }
                if path.is_empty() {
                    roots += 1;
                    if roots != 1 || e.name().as_ref() != "msg" {
                        return None;
                    }
                }
                if path.len() == 1 && e.name().as_ref() == "appmsg" {
                    apps += 1;
                }
                if path.len() == 2 && path[1] == "appmsg" && e.name().as_ref() == "type" {
                    types += 1;
                }
                path.push(e.name().as_ref().to_owned());
            }
            Event::End(e) => {
                if path.pop()?.as_str() != e.name().as_ref() {
                    return None;
                }
            }
            Event::Text(e) => {
                let text = e.xml10_content();
                if path.is_empty() && !text.trim().is_empty() {
                    return None;
                }
                if path.len() == 3 && path[1] == "appmsg" && path[2] == "type" {
                    value.push_str(&text);
                }
            }
            Event::CData(e) if !path.is_empty() => {
                if path.len() == 3 && path[1] == "appmsg" && path[2] == "type" {
                    value.push_str(&e.xml10_content());
                }
            }
            Event::GeneralRef(_) if !path.is_empty() => {
                if path.len() == 3 && path[1] == "appmsg" && path[2] == "type" {
                    return None;
                }
            }
            Event::Decl(_) if roots == 0 => {}
            Event::Comment(_) => {}
            Event::Eof => break,
            _ => return None,
        }
    }
    if !path.is_empty() || roots != 1 || apps != 1 || types != 1 {
        return None;
    }
    value.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtype_is_direct_unambiguous_and_sender_bound() {
        let xml = "<msg><appmsg><type>5</type><refermsg><type>6</type></refermsg></appmsg></msg>";
        assert_eq!(app_type(xml, None), Some(5));
        assert_eq!(app_type(&format!("owned:\n{xml}"), Some("owned")), Some(5));
        assert_eq!(app_type(&format!("other:\n{xml}"), Some("owned")), None);
        for xml in [
            "<msg><appmsg><type>5</type><type>6</type></appmsg></msg>",
            "<msg><appmsg><type>5</type></appmsg><appmsg><type>6</type></appmsg></msg>",
            "<msg><appmsg><refermsg><type>6</type></refermsg></appmsg></msg>",
            "<!DOCTYPE msg><msg><appmsg><type>5</type></appmsg></msg>",
            "<msg><appmsg><type>5</type></appmsg></msg><msg/>",
        ] {
            assert_eq!(app_type(xml, None), None);
        }
    }
}
