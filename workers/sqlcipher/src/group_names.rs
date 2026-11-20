//! Bounded protobuf room-member names; unknown layouts leave the optional field absent.
use std::collections::BTreeMap;
fn varint(input: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *input.get(*at)?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Some(value);
        }
    }
    None
}
fn fields(input: &[u8]) -> Option<Vec<(u32, Option<&[u8]>)>> {
    let mut at = 0;
    let mut output = Vec::new();
    while at < input.len() {
        if output.len() >= 20000 {
            return None;
        }
        let tag = varint(input, &mut at)?;
        let field = u32::try_from(tag >> 3).ok()?;
        if field == 0 || field > 0x1fffffff {
            return None;
        }
        let data = match tag & 7 {
            0 => {
                varint(input, &mut at)?;
                None
            }
            1 => {
                at = at.checked_add(8)?;
                None
            }
            5 => {
                at = at.checked_add(4)?;
                None
            }
            2 => {
                let len = usize::try_from(varint(input, &mut at)?).ok()?;
                let end = at.checked_add(len)?;
                let bytes = input.get(at..end)?;
                at = end;
                Some(bytes)
            }
            _ => return None,
        };
        if at > input.len() {
            return None;
        }
        output.push((field, data));
    }
    Some(output)
}
pub(super) fn decode(input: &[u8]) -> Option<BTreeMap<String, String>> {
    if input.is_empty() || input.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut names = BTreeMap::new();
    let mut identities = std::collections::BTreeSet::new();
    for (field, data) in fields(input)? {
        if field != 1 {
            continue;
        }
        let fields = fields(data?)?;
        let mut id = None;
        let mut name = None;
        for (number, value) in fields {
            if number == 1 || number == 2 {
                let text = std::str::from_utf8(value?).ok()?;
                if text.contains('\0') || text.len() > if number == 1 { 256 } else { 1024 } {
                    return None;
                }
                let target = if number == 1 { &mut id } else { &mut name };
                if target.replace(text).is_some() {
                    return None;
                }
            }
        }
        let id = id?;
        if id.is_empty() || id.chars().any(char::is_control) || !identities.insert(id.to_owned()) {
            return None;
        }
        if let Some(name) = name.filter(|s| !s.is_empty()) {
            names.insert(id.to_owned(), name.to_owned());
        }
    }
    if identities.is_empty() {
        None
    } else {
        Some(names)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn field(number: u8, value: &[u8]) -> Vec<u8> {
        let mut b = vec![(number << 3) | 2, value.len() as u8];
        b.extend(value);
        b
    }
    #[test]
    fn exact_names_and_unknown_fields() {
        let mut member = field(1, b"owned-member");
        member.extend(field(2, "群内名😀".as_bytes()));
        member.extend([24, 1]);
        let data = field(1, &member);
        assert_eq!(decode(&data).unwrap()["owned-member"], "群内名😀");
        let mut duplicate = data.clone();
        duplicate.extend(data);
        assert!(decode(&duplicate).is_none());
    }
    #[test]
    fn malformed_optional_blob_does_not_supply_names() {
        for input in [&[][..], &[10, 255], &[10, 2, 10, 0], &[255; 12], &[15, 1]] {
            assert!(decode(input).is_none());
        }
        assert!(decode(&vec![0; 2 * 1024 * 1024 + 1]).is_none());
    }
}
