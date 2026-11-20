//! Display-only records extracted from typed messages, never payment actions.
use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Transfer,
    RedPacket,
    FriendRequest,
    Call,
    Revoke,
    Link,
    Contact,
    Location,
    Forward,
    Quote,
    Share,
    System,
}
#[derive(Serialize)]
pub struct RecordDetail {
    pub kind: RecordKind,
    pub title: String,
    pub text: String,
    pub fields: BTreeMap<String, String>,
    pub url: Option<String>,
}
impl Drop for RecordDetail {
    fn drop(&mut self) {
        self.title.zeroize();
        self.text.zeroize();
        for value in self.fields.values_mut() { value.zeroize(); }
        self.url.zeroize();
    }
}
impl RecordDetail {
    fn new(kind: RecordKind, title: &str, text: &str) -> Self {
        Self {
            kind,
            title: title.into(),
            text: text.into(),
            fields: BTreeMap::new(),
            url: None,
        }
    }
    fn field(&mut self, name: &str, value: &str) {
        if !value.is_empty() {
            self.fields.insert(name.into(), value.into());
        }
    }
}
pub fn extract(kind: u32, body: &str) -> Result<Option<RecordDetail>, &'static str> {
    if body.len() > MAX_XML || body.contains('\0') {
        return Err(INVALID);
    }
    if kind == 37 {
        let (text,fields)=crate::special_records::request_detail(body).ok_or(INVALID)?;
        let mut detail=RecordDetail::new(RecordKind::FriendRequest,"好友验证记录",&text);
        detail.fields=fields;
        return Ok(Some(detail));
    }
    if matches!(kind, 10000 | 10002) {
        if body.trim_start().starts_with('<') {
            let root = document(body)?;
            if root.name != "sysmsg" { return Err(INVALID); }
            if let Some(revoke) = root.child("revokemsg")? {
                let mut detail =
                    RecordDetail::new(RecordKind::Revoke, "撤回通知", revoke.value("replacemsg")?);
                detail.field("原消息编号", revoke.value("newmsgid")?);
                detail.field("原会话", revoke.value("session")?);
                return Ok(Some(detail));
            }
            // Unknown system XML is not exposed as readable text/credentials.
            return Ok(Some(RecordDetail::new(
                RecordKind::System,
                "系统记录",
                root.value("content")?,
            )));
        }
        return Ok(Some(RecordDetail::new(
            RecordKind::System,
            "系统记录",
            body,
        )));
    }
    if kind == 50 {
        let text = if body.trim_start().starts_with('<') {
            let root = document(body)?;
            if root.children.is_empty() {
                root.text.to_string()
            } else {
                root.value("voipmsg")?.into()
            }
        } else {
            body.into()
        };
        return Ok(Some(RecordDetail::new(RecordKind::Call, "通话记录", &text)));
    }
    if kind == 49 {
        let root = document(body)?;
        if root.name != "msg" {
            return Err(INVALID);
        }
        let app = root.child("appmsg")?.ok_or(INVALID)?;
        let subtype = app
            .value("type")?
            .trim()
            .parse::<u32>()
            .map_err(|_| INVALID)?;
        if matches!(subtype, 2000 | 2001) {
            let mut detail = RecordDetail::new(
                if subtype == 2000 {
                    RecordKind::Transfer
                } else {
                    RecordKind::RedPacket
                },
                app.value("title")?,
                app.value("des")?,
            );
            if let Some(pay) = app.child("wcpayinfo")? {
                for (key, label) in [
                    ("feedesc", "消息显示金额"),
                    ("pay_memo", "备注"),
                    ("paysubtype", "原始状态码"),
                    ("receivertitle", "接收提示"),
                    ("sendertitle", "发送提示"),
                ] {
                    detail.field(label, pay.value(key)?);
                }
            }
            // No transaction tokens, native pay URLs, or inferred balance/status.
            return Ok(Some(detail));
        }
    }
    let Some(card) = parse_for_type(kind, body)? else {
        return Ok(None);
    };
    let detail = match &card {
        MessageCard::Contact { username, nickname } => {
            let mut d = RecordDetail::new(RecordKind::Contact, nickname, "");
            d.field("账号", username);
            d
        }
        MessageCard::Location {
            latitude,
            longitude,
            label,
            place_name,
        } => {
            let mut d = RecordDetail::new(RecordKind::Location, place_name, label);
            d.field("纬度", latitude);
            d.field("经度", longitude);
            d
        }
        MessageCard::Link {
            title,
            description,
            url,
        } => {
            let mut d = RecordDetail::new(RecordKind::Link, title, description);
            d.url = url.clone();
            d
        }
        MessageCard::AppShare {
            title,
            description,
            source_name,
            url,
            ..
        } => {
            let mut d = RecordDetail::new(RecordKind::Share, title, description);
            d.field("来源", source_name);
            d.url = url.clone();
            d
        }
        MessageCard::Quote {
            text,
            author,
            quoted_text,
            ..
        } => {
            let mut d = RecordDetail::new(RecordKind::Quote, "引用消息", text);
            d.field("被引用者", author);
            d.field("引用文字", quoted_text.as_deref().unwrap_or(""));
            d
        }
        MessageCard::Forward {
            title,
            entries,
            omitted,
        } => {
            let mut d = RecordDetail::new(RecordKind::Forward, title, "");
            d.field("可读条目", &entries.len().to_string());
            d.field("未展开条目", &omitted.to_string());
            d
        }
    };
    Ok(Some(detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friend_requests_share_strict_root_validation_and_never_expose_tickets() {
        let xml="<msg fromusername='owned-peer' fromnickname='自动化测试' content='验证 &amp; 说明' scene='6' opcode='2' status='0' ticket='not-public' encryptusername='private-name'/>";
        let detail=extract(37,xml).unwrap().unwrap();
        assert_eq!(detail.kind,RecordKind::FriendRequest);
        assert_eq!(detail.text,"验证 & 说明");
        assert_eq!(detail.fields["验证操作原码"],"2");
        assert_eq!(crate::special_records::request_text(xml).as_deref(),Some("验证 & 说明"));
        let public=serde_json::to_string(&detail).unwrap();
        assert!(!public.contains("not-public") && !public.contains("private-name"));
        for bad in ["<root><msg content='wrong-root'/></root>","<msg content='one'/><msg content='two'/>","<msg content='one' content='two'/>","<msg content='truncated'>","<!DOCTYPE msg [<!ENTITY e 'secret'>]><msg content='&e;'/>"] {
            assert!(extract(37,bad).is_err(),"{bad}");
            assert!(crate::special_records::request_text(bad).is_none());
        }
        assert_eq!(extract(37,"自动化测试：纯文字申请").unwrap().unwrap().text,"自动化测试：纯文字申请");
    }
    #[test]
    fn typed_records_do_not_expose_payment_credentials_or_guess_paid_state() {
        let xml = "<msg><appmsg><type>2000</type><title>转账</title><wcpayinfo><feedesc>￥1.00</feedesc><paysubtype>1</paysubtype><transferid>private-token</transferid><pay_memo>自动化测试</pay_memo></wcpayinfo></appmsg></msg>";
        let d = extract(49, xml).unwrap().unwrap();
        assert_eq!(d.kind, RecordKind::Transfer);
        assert_eq!(d.fields["消息显示金额"], "￥1.00");
        assert!(!serde_json::to_string(&d).unwrap().contains("private-token"));
        assert!(extract(1, xml).unwrap().is_none());
        assert!(
            extract(
                49,
                &xml.replace("<type>2000</type>", "<type>2000</type><type>2001</type>")
            )
            .is_err()
        );
    }
    #[test]
    fn revoke_and_calls_preserve_observations_without_inventing_original_content() {
        let d=extract(10002,"<sysmsg><revokemsg><newmsgid>18446744073709551615</newmsgid><replacemsg>撤回通知</replacemsg></revokemsg></sysmsg>").unwrap().unwrap();
        assert_eq!(d.kind, RecordKind::Revoke);
        assert_eq!(d.fields["原消息编号"], "18446744073709551615");
        assert_eq!(
            extract(50, "<voipmsg>已取消</voipmsg>")
                .unwrap()
                .unwrap()
                .text,
            "已取消"
        );
        assert!(
            extract(
                10002,
                "<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///secret'>]><sysmsg>&x;</sysmsg>"
            )
            .is_err()
        );
    }
}
