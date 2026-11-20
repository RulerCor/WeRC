//! Read only direct recipient metadata, without retaining the source envelope.
use quick_xml::{Reader, events::Event};

pub fn parse_mention_source(raw: &str) -> Option<Vec<String>> {
    if raw.len() > 16384 || raw.contains('\0') {
        return None;
    }
    let raw = raw.trim();
    let value = if raw.starts_with('{') {
        #[derive(serde::Deserialize)]
        struct Envelope {
            atuserlist: Option<serde_json::Value>,
        }
        let json: Envelope = serde_json::from_str(raw).ok()?;
        match json.atuserlist.as_ref() {
            None => String::new(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(items)) => {
                if items.len() > 256 {
                    return None;
                }
                items
                    .iter()
                    .map(|v| v.as_str())
                    .collect::<Option<Vec<_>>>()?
                    .join(",")
            }
            _ => return None,
        }
    } else {
        let mut reader = Reader::from_str(raw);
        reader.config_mut().expand_empty_elements = true;
        let mut path = Vec::<String>::new();
        let mut roots = 0;
        let mut lists = 0;
        let mut nodes = 0;
        let mut value = String::new();
        loop {
            match reader.read_event().ok()? {
                Event::Start(e) => {
                    nodes += 1;
                    if nodes > 2048 || path.len() >= 24 {
                        return None;
                    }
                    if path.is_empty() {
                        roots += 1;
                        if roots != 1 || e.name().as_ref() != "msgsource" {
                            return None;
                        }
                    }
                    if path.len() == 1 && e.name().as_ref() == "atuserlist" {
                        lists += 1;
                    }
                    if path.len() == 2 && path[1] == "atuserlist" {
                        return None;
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
                    if path.len() == 2 && path[1] == "atuserlist" {
                        value.push_str(&text);
                    }
                }
                Event::CData(e) if !path.is_empty() => {
                    if path.len() == 2 && path[1] == "atuserlist" {
                        value.push_str(&e.xml10_content());
                    }
                }
                Event::GeneralRef(_) if !path.is_empty() => {
                    if path.len() == 2 && path[1] == "atuserlist" {
                        return None;
                    }
                }
                Event::Comment(_) => {}
                Event::Decl(_) if roots == 0 => {}
                Event::Eof => break,
                _ => return None,
            }
        }
        if roots != 1 || lists > 1 || !path.is_empty() {
            return None;
        }
        value
    };
    let mut users = Vec::new();
    for user in value
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        if !super::valid_live_username(user) || users.len() >= 256 {
            return None;
        }
        if !users.iter().any(|s| s == user) {
            users.push(user.to_owned());
        }
    }
    Some(users)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recipients_are_exact_direct_and_never_guessed_from_content() {
        assert_eq!(
            parse_mention_source(
                "<msgsource><atuserlist><![CDATA[wxid_owned,wxid_owned_extra,notify@all,wxid_owned]]></atuserlist><ticket>secret</ticket></msgsource>"
            ),
            Some(vec![
                "wxid_owned".into(),
                "wxid_owned_extra".into(),
                "notify@all".into()
            ])
        );
        assert_eq!(
            parse_mention_source(r#"{"atuserlist":["owned","other"]}"#),
            Some(vec!["owned".into(), "other".into()])
        );
        assert_eq!(
            parse_mention_source(
                "<msgsource><refermsg><atuserlist>owned</atuserlist></refermsg></msgsource>"
            ),
            Some(vec![])
        );
        for raw in [
            "@owned",
            "<msgsource><atuserlist>owned</atuserlist><atuserlist>other</atuserlist></msgsource>",
            "<msgsource/><msgsource/>",
            "<!DOCTYPE msgsource><msgsource/>",
            "<msgsource><atuserlist><x>owned</x></atuserlist></msgsource>",
        ] {
            assert!(parse_mention_source(raw).is_none());
        }
    }
}
