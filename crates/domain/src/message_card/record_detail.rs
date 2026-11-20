//! Display fields for authenticated forwarded links and locations. No remote requests.
use super::{INVALID, MessageCard, Node, display_url};

pub(super) fn parse(item: &Node) -> Result<Option<MessageCard>, &'static str> {
    match item.data_type {
        Some(5) => {
            let web = item.child("weburlitem")?;
            let title = web.map(|n| n.value("title")).transpose()?.unwrap_or("");
            let title = if title.is_empty() {
                item.value("datatitle")?
            } else {
                title
            };
            let description = web.map(|n| n.value("desc")).transpose()?.unwrap_or("");
            let direct = item.value("link")?.trim();
            let nested = web
                .map(|n| n.value("link"))
                .transpose()?
                .unwrap_or("")
                .trim();
            if !direct.is_empty() && !nested.is_empty() && direct != nested {
                return Err(INVALID);
            }
            let url = display_url(if nested.is_empty() { direct } else { nested });
            if title.is_empty() && description.is_empty() && url.is_none() {
                return Ok(None);
            }
            Ok(Some(MessageCard::Link {
                title: title.into(),
                description: description.into(),
                url,
            }))
        }
        Some(6) => {
            let location = item.child("locitem")?.ok_or(INVALID)?;
            let latitude = location.value("lat")?.trim();
            let longitude = location.value("lng")?.trim();
            for (value, limit) in [(latitude, 90.0), (longitude, 180.0)] {
                let number = value.parse::<f64>().map_err(|_| INVALID)?;
                if !number.is_finite() || number.abs() > limit {
                    return Err(INVALID);
                }
            }
            Ok(Some(MessageCard::Location {
                latitude: latitude.into(),
                longitude: longitude.into(),
                label: location.value("label")?.into(),
                place_name: location.value("poiname")?.into(),
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    fn entries(body: &str) -> serde_json::Value {
        let xml = format!(
            "<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist>{body}</datalist></recordinfo>]]></recorditem></appmsg></msg>"
        );
        serde_json::to_value(super::super::parse(&xml).unwrap().unwrap()).unwrap()
    }
    #[test]
    fn forwarded_links_and_locations_keep_only_display_fields_at_each_level() {
        let link = "<dataitem datatype=\"5\"><link>https://example.test/?x=1&amp;y=2</link><weburlitem><title>A &amp; B</title><desc>&lt;b&gt;plain&lt;/b&gt;</desc><link>https://example.test/?x=1&amp;y=2</link></weburlitem><cdndatakey>secret</cdndatakey><sourceheadurl>private</sourceheadurl></dataitem>";
        let location = "<dataitem datatype=\"6\"><locitem><lat>31.25</lat><lng>121.5</lng><label>owned address</label><poiname>owned place</poiname><poiid>private</poiid></locitem></dataitem>";
        let nested = format!(
            "<dataitem datatype=\"17\"><recordxml><recordinfo><datalist>{link}{location}</datalist></recordinfo></recordxml></dataitem>"
        );
        let value = entries(&(link.to_owned() + location + &nested));
        let outer = &value["entries"];
        let inner = &outer[2]["nested"]["entries"];
        assert_eq!(outer[0]["detail"], inner[0]["detail"]);
        assert_eq!(outer[1]["detail"], inner[1]["detail"]);
        assert_eq!(outer[0]["detail"]["url"], "https://example.test/?x=1&y=2");
        assert_eq!(outer[0]["detail"]["description"], "<b>plain</b>");
        assert_eq!(outer[1]["detail"]["latitude"], "31.25");
        assert!(!value.to_string().contains("secret") && !value.to_string().contains("private"));
    }
    #[test]
    fn ambiguous_or_invalid_details_do_not_hide_the_record_or_siblings() {
        for body in [
            "<link>https://a.test</link><weburlitem><link>https://b.test</link></weburlitem>",
            "<weburlitem><title>a</title><title>b</title></weburlitem>",
            "<weburlitem><link>https://a.test</link></weburlitem><weburlitem/>",
        ] {
            let v = entries(&format!(
                "<dataitem datatype=\"5\">{body}</dataitem><dataitem datatype=\"1\"><datadesc>kept</datadesc></dataitem>"
            ));
            assert!(v["entries"][0].get("detail").is_none());
            assert_eq!(v["entries"][1]["text"], "kept");
        }
        for bad in ["NaN", "inf", "91", "-91", ""] {
            let v = entries(&format!(
                "<dataitem datatype=\"6\"><locitem><lat>{bad}</lat><lng>121</lng></locitem></dataitem>"
            ));
            assert!(v["entries"][0].get("detail").is_none());
        }
        let v = entries(
            "<dataitem datatype=\"5\"><datatitle>kept</datatitle><link>javascript:alert(1)</link></dataitem>",
        );
        assert_eq!(v["entries"][0]["detail"]["title"], "kept");
        assert!(v["entries"][0]["detail"]["url"].is_null());
    }
}
