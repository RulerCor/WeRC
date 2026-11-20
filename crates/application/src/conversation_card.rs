//! Display cards inherit the containing authenticated conversation view.
use serde::Serialize;
use werc_domain::{live::LiveMessage, message_card};
pub use werc_domain::message_card::DisplayContent as ConversationContent;

#[derive(Serialize)]
pub struct ConversationCard {
    pub local_id: String,
    pub source: Option<String>,
    pub content: ConversationContent,
}
pub(crate) fn card(row: &LiveMessage, source: Option<&str>) -> Option<ConversationCard> {
    let kind = (row.message_type.parse::<i64>().ok()? & 0xffffffff) as u32;
    if !matches!(kind, 37 | 42 | 48 | 49 | 50 | 10000 | 10002) {
        return None;
    }
    let body = row.metadata_body().or(row.text.as_deref())?;
    let content = message_card::display(kind,body)?;
    Some(ConversationCard {
        local_id: row.local_id.clone(),
        source: source.map(str::to_owned),
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sender_envelope_does_not_bypass_strict_card_validation() {
        let mut row = LiveMessage {
            at_user_list: None,
            local_id: "1".into(),
            server_id: None,
            timestamp: 1,
            message_type: "49".into(),
            text: None,
            metadata: None,
            sender_username: Some("owned".into()),
            content_status: "metadata".into(),
        };
        let xml = "<?xml version=\"1.0\"?><msg><appmsg><type>5</type><title>owned card</title></appmsg></msg>";
        row.metadata = Some(format!("owned:\n{xml}"));
        assert!(card(&row, Some("source-2")).is_some());
        for body in [
            format!("other:\n{xml}"),
            format!("owned:\n{xml}<msg/>"),
            format!("owned:\n<!DOCTYPE msg [<!ENTITY e 'owned'>]>{xml}"),
            "owned:\n<msg><appmsg>".into(),
        ] {
            row.metadata = Some(body);
            assert!(card(&row, None).is_none());
        }
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    #[test]
    fn system_call_friend_and_payment_cards_display_only_whitelisted_fields() {
        for (kind,body,category) in [
            (37,"<msg content='自动化测试' ticket='not-public'/>","friend_request"),
            (50,"<voipmsg>已取消</voipmsg>","call"),
            (10000,"自动化测试：系统通知","system"),
            (10002,"<sysmsg><revokemsg><newmsgid>9007199254740993</newmsgid><replacemsg>撤回通知</replacemsg></revokemsg></sysmsg>","revoke"),
            (49,"<msg><appmsg><type>2000</type><title>转账</title><wcpayinfo><paysubtype>3</paysubtype><transferid>not-public</transferid></wcpayinfo></appmsg></msg>","transfer"),
        ] {
            let row=LiveMessage { at_user_list:None,local_id:"1".into(),server_id:None,timestamp:1,message_type:kind.to_string(),text:None,metadata:Some(body.into()),sender_username:None,content_status:"metadata".into() };
            let value=serde_json::to_value(card(&row,Some("owned-source")).unwrap()).unwrap();
            assert_eq!(value["content"]["kind"],"record");
            assert_eq!(value["content"]["detail"]["kind"],category);
            assert!(!value.to_string().contains("not-public"));
        }
    }
    #[test]
    fn authenticated_type_sender_and_source_preserved_for_additional_cards() {
        for (kind, xml, expected) in [
            (
                42,
                r#"<msg username="owned-contact" nickname="Test"/>"#,
                "contact",
            ),
            (
                48,
                r#"<msg><location x="31" y="121" label="Test"/></msg>"#,
                "location",
            ),
            (
                49,
                r#"<msg><appmsg><type>33</type><title>Test</title></appmsg></msg>"#,
                "app_share",
            ),
        ] {
            let mut row = LiveMessage {
                at_user_list: None,
                local_id: "9007199254740993".into(),
                server_id: None,
                timestamp: 1,
                message_type: kind.to_string(),
                text: None,
                metadata: Some(format!("owned:\n{xml}")),
                sender_username: Some("owned".into()),
                content_status: "metadata".into(),
            };
            let result = card(&row, Some("source-2")).unwrap();
            let public = serde_json::to_value(result).unwrap();
            assert_eq!(public["content"]["kind"], expected);
            assert_eq!(public["source"], "source-2");
            assert_eq!(public["local_id"], row.local_id);
            row.sender_username = Some("other".into());
            assert!(card(&row, None).is_none());
            row.metadata = Some(xml.into());
            row.message_type = "1".into();
            assert!(card(&row, None).is_none());
        }
    }
}
