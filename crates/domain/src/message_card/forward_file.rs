//! A local-cache attachment is bound to the authenticated record's name, size,
//! and complete MD5. No paths or CDN credentials are accepted from a caller.
use super::{Node, record_tree};
use crate::{
    AppError, AppResult,
    file::{MAX_FILE_BYTES, validate_file_name},
};
use md5::{Digest, Md5};
use zeroize::Zeroizing;

pub struct ForwardFile {
    pub file_name: Zeroizing<String>,
    pub size: u64,
    md5: Zeroizing<String>,
}
fn invalid() -> AppError {
    AppError::new(
        "FORWARD_MEDIA_INVALID",
        "该转发条目没有可读取的完整附件信息",
    )
}
pub(super) fn from_item(item: &Node) -> AppResult<ForwardFile> {
    let value = |name| item.value(name).map_err(|_| invalid());
    let name = value("datatitle")?;
    let size = value("datasize")?;
    let hash = value("fullmd5")?;
    if item.name != "dataitem"
        || item.data_type != Some(8)
        || validate_file_name(name).is_err()
        || size.is_empty()
        || !size.bytes().all(|b| b.is_ascii_digit())
        || hash.len() != 32
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    let size = size.parse::<u64>().map_err(|_| invalid())?;
    if !(1..=MAX_FILE_BYTES as u64).contains(&size) {
        return Err(invalid());
    }
    Ok(ForwardFile {
        file_name: Zeroizing::new(name.into()),
        size,
        md5: Zeroizing::new(hash.to_ascii_lowercase()),
    })
}
pub fn descriptor(xml: &str, path: &[usize]) -> AppResult<ForwardFile> {
    record_tree::with_item(xml, path, |item| {
        from_item(item).map_err(|_| super::INVALID)
    })
    .map_err(|_| invalid())
}
impl ForwardFile {
    pub fn validate_bytes(&self, bytes: &[u8]) -> AppResult<()> {
        if bytes.len() as u64 != self.size || hex::encode(Md5::digest(bytes)) != *self.md5 {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file() -> String {
        "<dataitem datatype=\"8\"><datatitle>自动化测试.txt</datatitle><datasize>3</datasize><fullmd5>900150983CD24FB0D6963F7D28E17F72</fullmd5></dataitem>".into()
    }
    fn record(body: &str) -> String {
        format!("<recordinfo><datalist>{body}</datalist></recordinfo>")
    }
    fn nested(body: &str) -> String {
        format!(
            "<dataitem datatype=\"17\"><recordxml>{}</recordxml></dataitem>",
            record(body)
        )
    }
    fn xml(body: &str) -> String {
        format!(
            "<msg><appmsg><type>19</type><recorditem><![CDATA[{}]]></recorditem></appmsg></msg>",
            record(body)
        )
    }
    #[test]
    fn nested_selection_checks_exact_content_and_never_serializes_file_name_or_digest() {
        let escaped = record(&file())
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        for child in [
            nested(&file()),
            format!("<dataitem datatype=\"17\"><recordxml>{escaped}</recordxml></dataitem>"),
        ] {
            let body = xml(&(file() + &child));
            for path in [&[0][..], &[1, 0][..]] {
                let d = descriptor(&body, path).unwrap();
                assert_eq!(&*d.file_name, "自动化测试.txt");
                d.validate_bytes(b"abc").unwrap();
                assert!(d.validate_bytes(b"abd").is_err());
                assert!(d.validate_bytes(b"ab").is_err());
            }
            for path in [&[][..], &[1][..], &[0, 0][..], &[1, 1][..], &[100][..]] {
                assert!(descriptor(&body, path).is_err());
            }
            let public = serde_json::to_string(&super::super::parse(&body).unwrap()).unwrap();
            assert!(public.contains("\"file_available\":true"));
            assert!(!public.contains("自动化测试.txt"));
            assert!(!public.contains("900150"));
        }
    }
    #[test]
    fn traversal_respects_shared_budget_and_depth() {
        let body = xml(&(nested(&file().repeat(99)) + &file()));
        assert!(descriptor(&body, &[0, 98]).is_ok());
        assert!(descriptor(&body, &[1]).is_err());
        let body = xml(&nested(&nested(&nested(&nested(&file())))));
        assert!(descriptor(&body, &[0, 0, 0, 0, 0]).is_err());
        let body = xml(&nested(&nested(&nested(&file()))));
        assert!(descriptor(&body, &[0, 0, 0, 0]).is_ok());
    }
    #[test]
    fn invalid_names_sizes_duplicates_and_ambiguous_records_are_rejected() {
        for bad in [
            file().replace("自动化测试.txt", "../secret"),
            file().replace("自动化测试.txt", "CON.txt"),
            file().replace("<datasize>3", "<datasize>8388609"),
            file().replace("<datasize>3", "<datasize>+3"),
            file().replace("<datasize>3", "<datasize>0"),
            file().replace("</fullmd5>", "</fullmd5><fullmd5>00</fullmd5>"),
            file().replace("datatype=\"8\"", "datatype=\"2\""),
        ] {
            assert!(descriptor(&xml(&bad), &[0]).is_err());
        }
    }
}
