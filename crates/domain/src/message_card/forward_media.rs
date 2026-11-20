//! Credentials are selected from bounded authenticated XML, never from UI input.
use super::Node;
use crate::{
    AppError, AppResult,
    received_media::{RECEIVED_MEDIA_MAX_BYTES, ReceivedImageDescriptor},
};
use md5::{Digest, Md5};
use zeroize::Zeroizing;

pub struct ForwardImage {
    pub media: ReceivedImageDescriptor,
    md5: Zeroizing<String>,
}
pub enum ForwardMedia {
    Image(ForwardImage),
    File(super::forward_file::ForwardFile),
}
/// The authenticated record chooses the transport. A UI path never carries
/// credentials, file names, or an independently supplied media type.
pub fn descriptor(xml: &str, path: &[usize]) -> AppResult<ForwardMedia> {
    super::record_tree::with_item(xml, path, |item| match item.data_type {
        Some(2) => from_item(item)
            .map(ForwardMedia::Image)
            .map_err(|_| super::INVALID),
        Some(8) => super::forward_file::from_item(item)
            .map(ForwardMedia::File)
            .map_err(|_| super::INVALID),
        _ => Err(super::INVALID),
    })
    .map_err(|_| {
        AppError::new(
            "FORWARD_MEDIA_INVALID",
            "该转发条目没有可读取的完整图片或附件",
        )
    })
}
fn invalid() -> AppError {
    AppError::new("FORWARD_MEDIA_INVALID", "该转发条目没有可读取的完整图片")
}
fn hex_field(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value.len().is_multiple_of(2)
        && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn from_item(item: &Node) -> AppResult<ForwardImage> {
    let value = |name| item.value(name).map_err(|_| invalid());
    let key = value("cdndataurl")?;
    let aes = value("cdndatakey")?;
    let hash = value("fullmd5")?;
    let size = value("datasize")?;
    if item.name != "dataitem"
        || item.data_type != Some(2)
        || !hex_field(key, 2, 768)
        || !hex_field(aes, 32, 32)
        || !hex_field(hash, 32, 32)
        || !size.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let declared_size = size.parse::<u64>().map_err(|_| invalid())?;
    if !(1..=RECEIVED_MEDIA_MAX_BYTES).contains(&declared_size) {
        return Err(invalid());
    }
    Ok(ForwardImage {
        media: ReceivedImageDescriptor {
            file_key: Zeroizing::new(key.into()),
            aes_key: Zeroizing::new(aes.into()),
            declared_size,
        },
        md5: Zeroizing::new(hash.to_ascii_lowercase()),
    })
}
/// Zero-based dataitem index, identical to the displayed first 100 entries.
pub fn image_descriptor(xml: &str, index: usize) -> AppResult<ForwardImage> {
    super::record_tree::with_item(xml, &[index], |item| {
        from_item(item).map_err(|_| super::INVALID)
    })
    .map_err(|_| invalid())
}
impl ForwardImage {
    /// Verify the exact original object before any image normalization.
    pub fn validate_bytes(&self, bytes: &[u8]) -> AppResult<()> {
        if bytes.len() as u64 != self.media.declared_size
            || hex::encode(Md5::digest(bytes)) != *self.md5
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn nested(body: &str) -> String {
        format!(
            "<dataitem datatype=\"17\"><recordxml><recordinfo><datalist>{body}</datalist></recordinfo></recordxml></dataitem>"
        )
    }
    #[test]
    fn nested_media_selects_exact_type_and_credentials_without_flattening() {
        let inner = item().replace("aabb", "ffee");
        let body = xml(&(item() + &nested(&inner)));
        for (path, expected) in [(&[1][..], "aabb"), (&[2, 0][..], "ffee")] {
            let ForwardMedia::Image(image) = descriptor(&body, path).unwrap() else {
                panic!("wrong media type")
            };
            assert_eq!(&*image.media.file_key, expected);
            image.validate_bytes(b"abc").unwrap();
            assert!(image.validate_bytes(b"abd").is_err());
        }
        let public = serde_json::to_string(&super::super::parse(&body).unwrap()).unwrap();
        assert_eq!(public.matches("\"image_available\":true").count(), 2);
        assert!(!public.contains("ffee") && !public.contains("aabb"));
        for path in [
            &[][..],
            &[0][..],
            &[2][..],
            &[1, 0][..],
            &[2, 1][..],
            &[100][..],
        ] {
            assert!(descriptor(&body, path).is_err());
        }
        let file = "<dataitem datatype=\"8\"><datatitle>owned.txt</datatitle><datasize>3</datasize><fullmd5>900150983cd24fb0d6963f7d28e17f72</fullmd5></dataitem>";
        assert!(matches!(
            descriptor(&xml(&nested(file)), &[1, 0]).unwrap(),
            ForwardMedia::File(_)
        ));
    }
    #[test]
    fn nested_image_paths_share_display_depth_and_entry_budget() {
        let body = xml(&nested(&item().repeat(99)));
        // The preceding outer text and record group consume two entries.
        assert!(descriptor(&body, &[1, 97]).is_ok());
        assert!(descriptor(&body, &[1, 98]).is_err());
        assert!(descriptor(&xml(&nested(&nested(&nested(&item())))), &[1, 0, 0, 0]).is_ok());
        assert!(
            descriptor(
                &xml(&nested(&nested(&nested(&nested(&item()))))),
                &[1, 0, 0, 0, 0]
            )
            .is_err()
        );
    }
    fn xml(item: &str) -> String {
        format!(
            "<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist><ignored/><dataitem datatype=\"1\"><datadesc>hello</datadesc></dataitem>{item}</datalist></recordinfo>]]></recorditem></appmsg></msg>"
        )
    }
    fn item() -> String {
        format!(
            "<dataitem datatype=\"2\"><cdndataurl>aabb</cdndataurl><cdndatakey>{}</cdndatakey><datasize>3</datasize><fullmd5>900150983cd24fb0d6963f7d28e17f72</fullmd5></dataitem>",
            "ab".repeat(16)
        )
    }
    #[test]
    fn exact_entry_and_original_integrity() {
        let body = xml(&item());
        let d = image_descriptor(&body, 1).unwrap();
        d.validate_bytes(b"abc").unwrap();
        assert!(d.validate_bytes(b"abd").is_err());
        assert!(d.validate_bytes(b"abcd").is_err());
        for index in [0, 2, 100, usize::MAX] {
            assert!(image_descriptor(&body, index).is_err());
        }
        let public = serde_json::to_string(&super::super::parse(&body).unwrap()).unwrap();
        assert!(public.contains("\"image_available\":true"));
        for secret in ["aabb", "900150983cd24fb0d6963f7d28e17f72", &"ab".repeat(16)] {
            assert!(!public.contains(secret));
        }
    }
    #[test]
    fn wrong_types_duplicates_and_unbounded_values_rejected() {
        let good = item();
        for bad in [
            good.replace("datatype=\"2\"", "datatype=\"4\""),
            good.replace("aabb", "https://example.test/private"),
            good.replace("aabb", "abc"),
            good.replace("<datasize>3", "<datasize>8388609"),
            good.replace("<datasize>3", "<datasize>0"),
            good.replace("<datasize>3", "<datasize>+3"),
            good.replace("</cdndatakey>", "</cdndatakey><cdndatakey>00</cdndatakey>"),
            good.replace("<fullmd5>", "<fullmd5><nested/>"),
            good.replace("cdndataurl", "cdnthumburl"),
        ] {
            assert!(image_descriptor(&xml(&bad), 1).is_err());
        }
    }
}
