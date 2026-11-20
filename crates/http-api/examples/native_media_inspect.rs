//! Offline inspection of an isolated encrypted test store. Never prints values.
use quick_xml::{Reader, events::Event};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use werc_application::Application;
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

fn run() -> AppResult<()> {
    let path = PathBuf::from(std::env::args().nth(1).ok_or_else(AppError::internal)?);
    if !path.is_file() {
        return Err(AppError::internal());
    }
    let repo = Arc::new(SqliteRepository::open(&path)?);
    let app = Application::new(repo, Arc::new(WindowsWechatProvider));
    let accounts = app.accounts_local()?;
    if accounts.len() != 1 {
        return Err(AppError::internal());
    }
    let rows = app.received_texts_local(&accounts[0].id, 0, 100)?;
    let cursor = app
        .receive_cursor_local(&accounts[0].id)?
        .ok_or_else(AppError::internal)?;
    println!(
        "{}",
        serde_json::json!({"gaps":cursor.gaps,"dropped":cursor.dropped,"last_sequence":cursor.sequence})
    );
    let allowed = [
        "msg",
        "img",
        "videomsg",
        "voicemsg",
        "emoji",
        "appmsg",
        "appattach",
        "cdnmidimgurl",
        "cdnbigimgurl",
        "cdnthumburl",
        "aeskey",
        "length",
        "hdlength",
        "cdnthumblength",
        "cdnvideourl",
        "cdnvideoaeskey",
        "playlength",
        "voicelength",
        "voiceformat",
        "bufid",
        "clientmsgid",
        "md5",
        "cdnurl",
        "cdnattachurl",
        "voiceurl",
        "encrypturl",
        "externurl",
        "externmd5",
        "attachid",
        "totallen",
        "fileext",
        "type",
        "width",
        "height",
    ];
    let mut report = Vec::new();
    for row in &rows {
        let m = &row.payload.message;
        if m.message_type == 3 {
            let descriptor = werc_domain::received_media::image_descriptor(m)?;
            println!(
                "{}",
                serde_json::json!({"image_descriptor_valid":true,"declared_size":descriptor.declared_size,"media_downloaded":false})
            );
        }
        let mut fields = BTreeMap::new();
        let mut unknown_fields = 0;
        let mut stack = Vec::<String>::new();
        let mut scalar_lengths = BTreeMap::new();
        let mut appmsg_type = None;
        if let Some(xml) = &m.metadata {
            let mut reader = Reader::from_str(xml);
            let mut events = 0;
            loop {
                events += 1;
                if events > 4096 {
                    return Err(AppError::internal());
                }
                match reader
                    .read_event()
                    .map_err(|_| AppError::new("XML_INVALID", "invalid test metadata"))?
                {
                    event @ (Event::Start(_) | Event::Empty(_)) => {
                        let empty = matches!(event, Event::Empty(_));
                        let e = match event {
                            Event::Start(e) | Event::Empty(e) => e,
                            _ => unreachable!(),
                        };
                        let name = e.name();
                        if allowed.contains(&name.as_ref()) {
                            fields.insert(name.as_ref().to_owned(), 0);
                        } else {
                            unknown_fields += 1;
                        }
                        if !empty {
                            stack.push(name.as_ref().to_owned());
                        }
                        for attr in e.attributes() {
                            let attr = attr.map_err(|_| AppError::internal())?;
                            if allowed.contains(&attr.key.as_ref()) {
                                fields.insert(attr.key.as_ref().to_owned(), attr.value.len());
                            } else {
                                unknown_fields += 1;
                            }
                        }
                    }
                    Event::End(_) => {
                        stack.pop();
                    }
                    Event::Text(e) => {
                        if let Some(name) = stack.last().filter(|n| allowed.contains(&n.as_str())) {
                            scalar_lengths.insert(name.clone(), e.as_ref().len());
                            if stack.as_slice() == ["msg", "appmsg", "type"] {
                                appmsg_type =
                                    e.as_ref().parse::<u32>().ok().filter(|n| *n <= 100000);
                            }
                        }
                    }
                    Event::DocType(_) | Event::PI(_) => {
                        return Err(AppError::new(
                            "XML_UNSUPPORTED",
                            "unsupported test metadata",
                        ));
                    }
                    Event::Eof => break,
                    _ => {}
                }
            }
        }
        report.push(serde_json::json!({"type":m.message_type,"metadata_bytes":m.metadata.as_ref().map_or(0, String::len),"known_field_lengths":fields,"unknown_fields":unknown_fields,"known_scalar_lengths":scalar_lengths,"appmsg_type":appmsg_type}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|_| AppError::internal())?
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
