//! Native group mentions contain exact member IDs, never a display-name guess.
use crate::group::valid_group_username;

pub fn valid_members(group: &str, members: &[String]) -> bool {
    valid_group_username(group)
        && group.len() < 128
        && group
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@'))
        && (1..=8).contains(&members.len())
        && members.iter().enumerate().all(|(i, id)| {
            !id.is_empty()
                && id.len() < 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
                && !members[..i].contains(id)
        })
        && members.iter().map(String::len).sum::<usize>() + members.len() - 1 < 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_group_unknown_all_duplicates_and_unbounded_lists() {
        let good = vec!["member-a".into(), "long_member_1234567890".into()];
        assert!(valid_members("owned@chatroom", &good));
        assert!(!valid_members("filehelper", &good));
        for invalid in [
            vec![],
            vec!["".into()],
            vec!["@all".into()],
            vec!["notify@all".into()],
            vec!["a,b".into()],
            vec!["a".into(), "a".into()],
            vec!["x".repeat(128)],
            (0..9).map(|i| format!("member-{i}")).collect(),
        ] {
            assert!(!valid_members("owned@chatroom", &invalid));
        }
        let limit = (0..8)
            .map(|i| format!("{i}{}", "x".repeat(126)))
            .collect::<Vec<_>>();
        assert!(valid_members("owned@chatroom", &limit));
    }
}
