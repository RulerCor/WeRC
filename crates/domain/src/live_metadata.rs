//! Source-specific metadata decoding above the unchanged worker wire contract.
use crate::live::{LiveMessage, valid_live_username};

impl LiveMessage {
    /// Some source rows prefix XML with the sender resolved independently from
    /// Name2Id. Remove only that exact envelope; never infer identity from XML.
    /// Callers must still parse the returned body with their strict XML parser.
    pub fn metadata_body(&self) -> Option<&str> {
        let raw = self.metadata.as_deref()?;
        let body = self
            .sender_username
            .as_deref()
            .filter(|sender| valid_live_username(sender))
            .and_then(|sender| raw.strip_prefix(sender))
            .and_then(|suffix| suffix.strip_prefix(":\n"))
            .filter(|body| body.starts_with('<'));
        Some(body.unwrap_or(raw))
    }
}

#[cfg(test)]
mod envelope_tests {
    use super::*;

    #[test]
    fn xml_envelope_requires_exact_independent_sender_and_preserves_raw_message() {
        let xml =
            "<?xml version=\"1.0\"?><msg><appmsg><type>5</type><title>owned</title></appmsg></msg>";
        let mut row = LiveMessage {
            at_user_list: None,
            local_id: "1".into(),
            server_id: None,
            timestamp: 1,
            message_type: "49".into(),
            text: None,
            metadata: None,
            sender_username: Some("owned-sender".into()),
            content_status: "metadata".into(),
        };
        assert_eq!(row.metadata_body(), None);
        for body in [xml, "<msg/>"] {
            row.metadata = Some(format!("owned-sender:\n{body}"));
            let before = serde_json::to_vec(&row).unwrap();
            assert_eq!(row.metadata_body(), Some(body));
            assert_eq!(serde_json::to_vec(&row).unwrap(), before);
            row.metadata = Some(body.into());
            assert_eq!(row.metadata_body(), Some(body));
        }
        for raw in [
            format!("other-sender:\n{xml}"),
            format!("owned-sender-long:\n{xml}"),
            format!("owned-sender:\r\n{xml}"),
            "owned-sender:\nordinary text".to_owned(),
            format!("owned-sender:\nother-sender:\n{xml}"),
        ] {
            row.metadata = Some(raw.clone());
            assert_eq!(row.metadata_body(), Some(raw.as_str()));
        }
        row.metadata = Some(format!("owned-sender:\n{xml}"));
        for sender in [None, Some(String::new()), Some("other-sender".into())] {
            row.sender_username = sender;
            assert_eq!(row.metadata_body(), row.metadata.as_deref());
        }
    }
}
