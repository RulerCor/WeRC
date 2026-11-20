//! Whitelisted local general.db records. Tickets, payload blobs and payment URLs stay out.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;
pub const MAX_ROWS: usize = 100;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSource {
    FriendRequest,
    SearchChat,
    SearchWeb,
    SearchBrand,
    Revoke,
    Transfer,
    RedPacket,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordQuery {
    pub source: RecordSource,
    #[serde(default)]
    pub text: String,
    pub from: Option<i64>,
    pub until: Option<i64>,
}
impl RecordQuery {
    pub fn valid(&self) -> bool {
        self.text.len() <= 512
            && !self.text.contains('\0')
            && self.from.is_none_or(|v| v >= 0)
            && self.until.is_none_or(|v| v >= 0)
            && !matches!((self.from,self.until),(Some(a),Some(b)) if a>=b)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordAnchor {
    pub row_id: String,
    pub raw_time: i64,
}
impl RecordAnchor {
    pub fn key(&self) -> Option<(i64, i64)> {
        let row = self.row_id.parse::<i64>().ok()?;
        (row > 0 && row.to_string() == self.row_id).then_some((self.raw_time, row))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordSelection {
    pub query: RecordQuery,
    pub after: Option<RecordAnchor>,
}
impl RecordSelection {
    pub fn valid(&self) -> bool {
        self.query.valid() && self.after.as_ref().is_none_or(|a| a.key().is_some())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecialRecord {
    pub anchor: RecordAnchor,
    pub source: RecordSource,
    pub timestamp: Option<i64>,
    pub username: Option<String>,
    pub text: String,
    pub direction: String,
    pub fields: std::collections::BTreeMap<String, String>,
}
impl SpecialRecord {
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some()
            && self
                .timestamp
                .is_none_or(|v| (0..=253402300799).contains(&v))
            && self.username.as_ref().is_none_or(|s| safe(s, 256))
            && safe(&self.text, 8192)
            && matches!(self.direction.as_str(), "incoming" | "outgoing" | "unknown")
            && self.fields.len() <= 12
            && self.fields.iter().all(|(k, v)| safe(k, 64) && safe(v, 512))
    }
}
impl Drop for SpecialRecord {
    fn drop(&mut self) {
        self.username.zeroize();
        self.text.zeroize();
        for v in self.fields.values_mut() {
            v.zeroize();
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordWindow {
    pub rows: Vec<SpecialRecord>,
    pub next: Option<RecordAnchor>,
    pub scanned: usize,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl RecordWindow {
    pub fn valid(&self, s: &RecordSelection) -> bool {
        if !s.valid()
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > MAX_ROWS
            || self.scanned > 300
            || self.scanned < self.rows.len()
        {
            return false;
        }
        let mut last = s.after.as_ref().and_then(RecordAnchor::key);
        for row in &self.rows {
            let Some(k) = row.anchor.key() else {
                return false;
            };
            if !row.valid() || row.source != s.query.source || last.is_some_and(|a| k >= a) {
                return false;
            }
            last = Some(k);
        }
        self.next.as_ref().is_none_or(|n| {
            n.key().is_some_and(|k| {
                s.after
                    .as_ref()
                    .and_then(RecordAnchor::key)
                    .is_none_or(|a| k < a)
                    && last.is_none_or(|a| k <= a)
            })
        })
    }
}
fn safe(s: &str, limit: usize) -> bool {
    s.len() <= limit
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}
pub fn text(value: &str, limit: usize) -> Option<String> {
    safe(value, limit).then(|| value.to_owned())
}
pub fn request_text(value: &str) -> Option<String> {
    request_detail(value).map(|(body,_)| body)
}
/// Shared by the standalone SQLCipher worker and message-card projection. Only
/// direct root attributes are public; the complete bounded document must parse.
pub fn request_detail(value: &str) -> Option<(String, std::collections::BTreeMap<String,String>)> {
    use quick_xml::{Reader, events::Event};
    if value.len()>65536 || value.contains('\0') {return None;}
    if !value.trim_start().starts_with('<') {return Some((text(value,8192)?,Default::default()));}
    let mut reader=Reader::from_str(value);reader.config_mut().expand_empty_elements=true;
    let mut depth=0usize;let mut roots=0;let mut nodes=0;let mut declaration=false;
    let mut body=String::new();let mut fields=std::collections::BTreeMap::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e)=>{
                nodes+=1;if nodes>4096 || depth>=32{return None;}
                if depth==0 {
                    roots+=1;if roots!=1 || e.name().as_ref()!="msg" {return None;}
                    for (i,a) in e.attributes().enumerate() {
                        if i>=64{return None;}let a=a.ok()?;
                        let label=match a.key.as_ref(){"content"=>"","fromusername"=>"申请账号","fromnickname"=>"申请昵称","scene"=>"来源场景原码","opcode"=>"验证操作原码","status"=>"验证状态原码",_=>continue};
                        let v=a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok()?;
                        let v=text(&v,if label.is_empty(){8192}else{512})?;
                        if label.is_empty(){body=v;}else if !v.is_empty(){fields.insert(label.into(),v);}
                    }
                }
                depth+=1;
            }
            Event::End(_)=>{depth=depth.checked_sub(1)?;}
            Event::Text(e)=>{if depth==0 && !e.xml10_content().trim().is_empty(){return None;}}
            Event::CData(_) | Event::GeneralRef(_) if depth>0=>{}
            Event::Comment(_)=>{}
            Event::Decl(_) if roots==0 && depth==0 && !declaration=>{declaration=true;}
            Event::Eof=>return (roots==1 && depth==0).then_some((body,fields)),
            _=>return None,
        }
    }
}
