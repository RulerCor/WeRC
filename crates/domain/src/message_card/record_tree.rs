//! Shared entry budget across nested records; no additional remote loading.
use super::{INVALID, NestedRecord, Node, RecordEntry, document, forward_file, forward_media};

pub(super) fn entries(
    record: &Node,
    depth: usize,
    remaining: &mut usize,
) -> Result<NestedRecord, &'static str> {
    if record.name != "recordinfo" {
        return Err(INVALID);
    }
    let list = record.child("datalist")?.ok_or(INVALID)?;
    let mut result = NestedRecord {
        entries: Vec::new(),
        omitted: 0,
        depth_limited: depth >= 4,
    };
    for item in list.children.iter().filter(|n| n.name == "dataitem") {
        if *remaining == 0 || result.depth_limited {
            result.omitted += 1;
            continue;
        }
        *remaining -= 1;
        let data_type = item.data_type.ok_or(INVALID)?;
        let nested = if data_type == 17 {
            match item.child("recordxml")? {
                Some(xml) if xml.children.is_empty() => {
                    let node = document(&xml.text)?;
                    Some(entries(&node, depth + 1, remaining)?)
                }
                Some(xml) if xml.text.trim().is_empty() && xml.children.len() == 1 => {
                    Some(entries(&xml.children[0], depth + 1, remaining)?)
                }
                Some(_) => return Err(INVALID),
                None => None,
            }
        } else {
            None
        };
        result.entries.push(RecordEntry {
            image_available: forward_media::from_item(item).is_ok(),
            file_available: forward_file::from_item(item).is_ok(),
            detail: super::record_detail::parse(item)
                .ok()
                .flatten()
                .map(Box::new),
            author: item.value("sourcename")?.into(),
            timestamp: item.value("sourcetime")?.into(),
            data_type,
            text: if data_type == 1 {
                Some(item.value("datadesc")?.into())
            } else {
                None
            },
            nested,
        });
    }
    Ok(result)
}

/// Select only entries represented in the displayed tree, including the shared
/// budget consumed by preceding nested groups. Indices are never flattened.
pub(super) fn with_item<T>(
    xml: &str,
    path: &[usize],
    apply: impl FnOnce(&Node) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if path.is_empty() || path.len() > 4 || path.iter().any(|i| *i >= 100) {
        return Err(INVALID);
    }
    let root = document(xml)?;
    if root.name != "msg" {
        return Err(INVALID);
    }
    let app = root.child("appmsg")?.ok_or(INVALID)?;
    if app.value("type")?.trim() != "19" {
        return Err(INVALID);
    }
    let record = document(app.value("recorditem")?)?;
    let tree = entries(&record, 0, &mut 100)?;
    let mut shown = &tree;
    for (level, index) in path.iter().enumerate() {
        let entry = shown.entries.get(*index).ok_or(INVALID)?;
        if level + 1 < path.len() {
            shown = entry.nested.as_ref().ok_or(INVALID)?;
        }
    }
    walk(&record, path, apply)
}
fn walk<T>(
    record: &Node,
    path: &[usize],
    apply: impl FnOnce(&Node) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let item = record
        .child("datalist")?
        .ok_or(INVALID)?
        .children
        .iter()
        .filter(|n| n.name == "dataitem")
        .nth(path[0])
        .ok_or(INVALID)?;
    if path.len() == 1 {
        return apply(item);
    }
    if item.data_type != Some(17) {
        return Err(INVALID);
    }
    let xml = item.child("recordxml")?.ok_or(INVALID)?;
    if xml.children.is_empty() {
        walk(&document(&xml.text)?, &path[1..], apply)
    } else if xml.text.trim().is_empty() && xml.children.len() == 1 {
        walk(&xml.children[0], &path[1..], apply)
    } else {
        Err(INVALID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(item: &str) -> String {
        format!("<recordinfo><datalist>{item}</datalist></recordinfo>")
    }
    fn text() -> String {
        "<dataitem datatype=\"1\"><sourcename>owned</sourcename><datadesc>hello &amp; world</datadesc></dataitem>".into()
    }
    fn nested(body: &str) -> String {
        format!("<dataitem datatype=\"17\"><recordxml>{body}</recordxml></dataitem>")
    }
    #[test]
    fn inline_and_escaped_records_parse_without_transport_fields() {
        for child in [
            root(&text()),
            root(&text())
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        ] {
            let doc = document(&root(&nested(&child))).unwrap();
            let parsed = entries(&doc, 0, &mut 100).unwrap();
            let child = parsed.entries[0].nested.as_ref().unwrap();
            assert_eq!(child.entries[0].text.as_deref(), Some("hello & world"));
            assert!(!child.depth_limited);
            assert_eq!(child.omitted, 0);
        }
    }
    #[test]
    fn one_budget_spans_the_entire_tree_and_depth_is_explicit() {
        let doc = document(&root(&(nested(&root(&text().repeat(103))) + &text()))).unwrap();
        let parsed = entries(&doc, 0, &mut 100).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.omitted, 1);
        let child = parsed.entries[0].nested.as_ref().unwrap();
        assert_eq!(child.entries.len(), 99);
        assert_eq!(child.omitted, 4);
        let mut body = root(&text());
        for _ in 0..5 {
            body = root(&nested(&body));
        }
        let doc = document(&body).unwrap();
        let parsed = entries(&doc, 0, &mut 100).unwrap();
        let mut node = &parsed;
        for _ in 0..4 {
            node = node.entries[0].nested.as_ref().unwrap();
        }
        assert!(node.depth_limited);
        assert_eq!(node.omitted, 1);
        assert!(node.entries.is_empty());
    }
    #[test]
    fn nested_ambiguity_invalid_roots_and_entities_remain_rejected() {
        for body in [
            "<wrong/>",
            "<recordinfo/><recordinfo/>",
            "mixed<recordinfo/>",
            "&lt;!DOCTYPE recordinfo&gt;&lt;recordinfo/&gt;",
        ] {
            let doc = document(&root(&nested(body))).unwrap();
            assert!(entries(&doc, 0, &mut 100).is_err());
        }
    }
}
