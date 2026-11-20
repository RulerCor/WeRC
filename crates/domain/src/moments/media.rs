//! Transport-only media descriptors, resolved from an authenticated SNS row.
//! Never include these URLs/keys in timeline responses, exports or logs.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentMediaVariant {
    Original,
    Thumbnail,
    LiveVideo,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentMediaSelection {
    pub anchor: LiveMomentAnchor,
    pub slot: usize,
    pub variant: MomentMediaVariant,
}
impl MomentMediaSelection {
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some() && self.slot < 256
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MomentMediaDescriptor {
    pub moment: LiveMoment,
    pub url: String,
    pub token: String,
    pub seed: Option<u64>,
    pub md5: Option<String>,
    pub video: bool,
}
impl Drop for MomentMediaDescriptor {
    fn drop(&mut self) {
        self.url.zeroize();
        self.token.zeroize();
        self.seed.zeroize();
        self.md5.zeroize();
    }
}
pub fn media_descriptor(
    xml: &str,
    selection: &MomentMediaSelection,
) -> Result<MomentMediaDescriptor, &'static str> {
    const INVALID: &str = "MOMENT_MEDIA_INVALID";
    if !selection.valid() {
        return Err(INVALID);
    }
    let observed = parse_content(xml, &selection.anchor)?;
    let root = details::tree(xml)?;
    let timeline = root.child("TimelineObject")?.ok_or(INVALID)?;
    let content = timeline.child("ContentObject")?.ok_or(INVALID)?;
    let list = content.child("mediaList")?.ok_or("MOMENT_MEDIA_MISSING")?;
    let media = list
        .children
        .iter()
        .filter(|n| n.name == "media")
        .nth(selection.slot)
        .ok_or("MOMENT_MEDIA_MISSING")?;
    let media = if selection.variant == MomentMediaVariant::LiveVideo {
        media
            .child("LivePhoto")?
            .and_then(|n| n.child("liveMedia").ok().flatten())
            .ok_or("MOMENT_MEDIA_MISSING")?
    } else {
        media
    };
    let video = selection.variant != MomentMediaVariant::Thumbnail
        && (selection.variant == MomentMediaVariant::LiveVideo || media.value("type")? == "6");
    let node = media
        .child(if selection.variant == MomentMediaVariant::Thumbnail {
            "thumb"
        } else {
            "url"
        })?
        .ok_or("MOMENT_MEDIA_MISSING")?;
    if node.text.is_empty() || node.text.len() > 8192 {
        return Err("MOMENT_MEDIA_MISSING");
    }
    let raw_seed = if video {
        media
            .child("enc")?
            .and_then(|n| n.attributes.get("key"))
            .or_else(|| node.attributes.get("key"))
    } else {
        node.attributes.get("key")
    };
    let seed = raw_seed
        .filter(|v| !v.is_empty())
        .map(|v| v.parse::<u64>().map_err(|_| INVALID))
        .transpose()?
        .filter(|v| *v != 0);
    let token = node.attributes.get("token").cloned().unwrap_or_default();
    if token.len() > 4096 {
        return Err(INVALID);
    }
    let md5 = node
        .attributes
        .get(if video { "videomd5" } else { "md5" })
        .or_else(|| node.attributes.get("md5"))
        .filter(|v| !v.is_empty())
        .cloned();
    if md5
        .as_ref()
        .is_some_and(|v| v.len() != 32 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(INVALID);
    }
    Ok(MomentMediaDescriptor {
        moment: LiveMoment { anchor: selection.anchor.clone(), content: Some(observed), content_status: "parsed".into() },
        url: node.text.to_string(),
        token,
        seed,
        md5,
        video,
    })
}

pub struct MomentMediaBytes {
    pub moment: LiveMoment,
    pub bytes: Zeroizing<Vec<u8>>,
    pub mime: &'static str,
    pub extension: &'static str,
    pub source_md5_matches: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_media_selection_preserves_transport_credentials_only() {
        let xml = r#"<SnsDataItem><TimelineObject><id>19</id><username>owned</username><createTime>1700000000</createTime><contentDesc>自动化测试</contentDesc><ContentObject><contentStyle>1</contentStyle><mediaList><media><type>2</type><url key="18446744073709551615" token="owned%2Ftoken" md5="11111111111111111111111111111111">https://mmsns.qpic.cn/owned/150</url><thumb key="23">https://mmsns.qpic.cn/owned-thumb</thumb><LivePhoto><liveMedia><type>6</type><url>https://owned.video.qq.com/clip</url><enc key="456"/></liveMedia></LivePhoto></media></mediaList></ContentObject></TimelineObject></SnsDataItem>"#;
        let mut selection = MomentMediaSelection {
            anchor: LiveMomentAnchor {
                id: "19".into(),
                row_id: "1".into(),
                username: "owned".into(),
            },
            slot: 0,
            variant: MomentMediaVariant::Original,
        };
        let d = media_descriptor(xml, &selection).unwrap();
        assert_eq!(d.seed, Some(u64::MAX));
        assert_eq!(d.token, "owned%2Ftoken");
        assert!(!d.video);
        let visible =
            serde_json::to_string(&parse_content(xml, &selection.anchor).unwrap()).unwrap();
        assert!(!visible.contains("owned%2Ftoken"));
        assert!(!visible.contains("18446744073709551615"));
        assert!(!visible.contains("mmsns.qpic"));
        selection.variant = MomentMediaVariant::Thumbnail;
        assert_eq!(media_descriptor(xml, &selection).unwrap().seed, Some(23));
        selection.variant = MomentMediaVariant::LiveVideo;
        let d = media_descriptor(xml, &selection).unwrap();
        assert!(d.video);
        assert_eq!(d.seed, Some(456));
        selection.anchor.username = "different".into();
        assert!(media_descriptor(xml, &selection).is_err());
        selection.anchor.username = "owned".into();
        selection.slot = 1;
        assert!(media_descriptor(xml, &selection).is_err());
    }
}
