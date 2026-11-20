#![cfg(windows)]
#[path="support/owned_directory.rs"]
mod owned_directory;
#[path = "live_read/backfill.rs"]
mod backfill;
#[path = "live_read/calendar.rs"]
mod calendar;
#[path = "live_read/concurrency.rs"]
mod concurrency;
#[path = "live_read/capture.rs"]
mod captures;
#[path = "live_read/client_quote.rs"]
mod client_quote;
#[path = "live_read/search.rs"]
mod cross_search;
#[path = "live_read/directory_export.rs"]
mod directory_exports;
#[path = "live_read/exact_receipts.rs"]
mod exact_receipts;
#[path = "live_read/favorites.rs"]
mod favorites;
#[path = "live_read/forward_paths.rs"]
mod forward_paths;
#[path = "live_read/live_export.rs"]
mod live_exports;
#[path = "live_read/moment_media.rs"]
mod moment_media;
#[path = "live_read/moment_preservation.rs"]
mod moment_preservation;
#[path = "live_read/articles.rs"]
mod articles;
#[path = "live_read/special_records.rs"]
mod special_records;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use werc_application::{
    Application,
    live_view::LiveDirection,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::moments::*;
use werc_domain::{AppError, AppResult, capability::Capability, connection::*, live::*, outbox::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct Reader {
    friend_fixture: AtomicBool,
    auto_sources: Mutex<Vec<String>>,
    context: Mutex<SendContext>,
    engine: Mutex<String>,
    salt: Mutex<[u8; 16]>,
    reads: AtomicUsize,
    change: AtomicBool,
    wait: AtomicBool,
    body: Mutex<String>,
    spread_time: AtomicBool,
    voice: AtomicBool,
    voice_missing: AtomicBool,
    legacy_sender: AtomicBool,
    invalid_group: AtomicBool,
    animation_error: AtomicBool,
    last_voice: Mutex<Option<(String, String)>>,
    recipients: Mutex<Vec<String>>,
    quote_calls: AtomicUsize,
    quote_source_ids: Mutex<Vec<String>>,
    quote_submit_pause: Mutex<Option<QuoteSubmitPause>>,
    quote_observation: Mutex<Option<(werc_domain::receive::ReceivedRoute, werc_domain::receive::ReceivedText)>>,
    compressed_text: AtomicBool,
    sender_envelope: AtomicBool,
    moment_media_enabled: AtomicBool,
    on_unprotect: Mutex<Option<(String, Box<dyn FnOnce() + Send>)>>,
}
type QuoteSubmitPause = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
impl CapabilityProvider for Reader {
    fn decode_image_cache(&self,sample:&[u8],key:&ImageCacheKey,cancel:&AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{WindowsWechatProvider.decode_image_cache(sample,key,cancel)}
    fn recover_image_cache_key(&self,_account:&str,sample:&[u8],cancel:&AtomicBool)->AppResult<ImageCacheKey>{
        let key=ImageCacheKey{aes:None,xor:Some(0x42)};WindowsWechatProvider.decode_image_cache(sample,&key,cancel)?;Ok(key)
    }
    fn fetch_contact_avatar(&self,_account:&str,url:&str,cancel:&AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{
        assert_eq!(url,"https://wx.qlogo.cn/owned-avatar");
        if cancel.load(Ordering::SeqCst){return Err(AppError::new("QUERY_CANCELED","owned"));}
        Ok(werc_domain::image::canonical_png(include_bytes!("../../../tests/fixtures/owned-image.png"))?.1)
    }
    fn read_live_contacts_filtered(&self,account:&str,database:&str,fingerprint:&str,after:Option<&LiveContactContinuation>,query:Option<&str>,cancel:&AtomicBool)->AppResult<(String,LiveContactWindow)>{
        if query.is_none(){return self.read_live_contacts(account,database,fingerprint,after,cancel);}
        if query == Some("@chatroom") {
            let (fp, mut page) = self.read_live_contacts(account, database, fingerprint, None, cancel)?;
            page.rows.truncate(3);
            page.has_more = false;
            page.rows[0].anchor.username = "owned@chatroom".into();
            page.rows[0].remark = Some("Owned group".into());
            page.rows[1].anchor.username = "deleted@chatroom".into();
            page.rows[1].delete_flag = Some(1);
            return Ok((fp, page));
        }
        let(fp,mut page)=self.read_live_contacts(account,database,fingerprint,None,cancel)?;
        let boundary=LiveContactContinuation{source_fingerprint:fp.clone(),anchor:page.rows.last().unwrap().anchor.clone(),search_query:None};
        page.rows.extend(self.read_live_contacts(account,database,fingerprint,Some(&boundary),cancel)?.1.rows);
        page.rows.retain(|r|r.matches_query(query.unwrap())&&after.is_none_or(|a|r.anchor.key()>a.anchor.key()));
        page.has_more=page.rows.len()>200;page.rows.truncate(200);Ok((fp,page))
    }
    fn article_history(&self,_account:&str,publisher:&str,_cursor:Option<&str>,_cancel:&AtomicBool)->AppResult<werc_domain::article_history::ArticleHistoryPage>{
        if publisher=="gh_missing"{return Err(AppError::new("ARTICLE_HISTORY_SERVER_ERROR","owned"));}
        Ok(werc_domain::article_history::ArticleHistoryPage{publisher:publisher.into(),data:vec![werc_domain::article_history::ArticleSummary{url:"https://mp.weixin.qq.com/s/owned".into(),title:"自动化测试".into(),published_at:None}],next_cursor:None,end_reached:true,skipped:0,source_mode:"native_authenticated".into()})
    }

    fn fetch_article_image(&self,_account:&str,_url:&str,cancel:&AtomicBool)->AppResult<werc_domain::articles::ArticleImageBytes>{
        if cancel.load(Ordering::SeqCst){return Err(AppError::new("QUERY_CANCELED","owned"));}
        let bytes=include_bytes!("../../../tests/fixtures/owned-image.png");
        let (_,bytes)=werc_domain::image::canonical_png(bytes)?;
        Ok(werc_domain::articles::ArticleImageBytes{bytes,mime:"image/png".into(),extension:"png".into()})
    }
    fn fetch_article(&self,_account:&str,url:&str,cancel:&AtomicBool)->AppResult<werc_domain::articles::Article>{
        while self.wait.load(Ordering::SeqCst){if cancel.load(Ordering::SeqCst){return Err(AppError::new("QUERY_CANCELED","owned cancellation"));}std::thread::sleep(std::time::Duration::from_millis(5));}
        if url.ends_with("/missing"){return Err(AppError::new("ARTICLE_LOGIN_REQUIRED","自动化测试登录失效"));}
        let body=self.body.lock().unwrap().clone();
        werc_domain::articles::parse_article(url,&format!("<h1 id='activity-name'>自动化测试文章</h1><span id='js_name'>自有样本</span><div id='js_content'><p>{body}</p><img data-src='https://mmbiz.qpic.cn/owned'></div>"),"public_web",1)
    }
    fn automatic_message_sources(
        &self,
        _account: &str,
    ) -> AppResult<werc_application::ports::AutomaticMessageSources> {
        Ok(werc_application::ports::AutomaticMessageSources {
            process_id: 1,
            databases: self.auto_sources.lock().unwrap().clone(),
        })
    }

    fn read_live_special_records(
        &self,
        _account: &str,
        database: &str,
        fingerprint: &str,
        selection: &werc_domain::special_records::RecordSelection,
        expected: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, werc_domain::special_records::RecordWindow)> {
        use werc_domain::special_records::*;
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "owned"));
        }
        if let Some(e) = expected {
            assert_eq!(e, "e".repeat(64));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-record".into();
        }
        let n = if selection.after.is_some() { 1 } else { 2 };
        let anchor = RecordAnchor {
            row_id: n.to_string(),
            raw_time: 1700000000,
        };
        Ok((
            "e".repeat(64),
            RecordWindow {
                rows: vec![SpecialRecord {
                    anchor: anchor.clone(),
                    source: selection.query.source,
                    timestamp: Some(1700000000),
                    username: Some("owned".into()),
                    text: "=自动化测试".into(),
                    direction: "incoming".into(),
                    fields: Default::default(),
                }],
                next: (n == 2).then_some(anchor),
                scanned: 1,
                read_only: true,
                readonly_shm: true,
            },
        ))
    }
    fn read_live_favorites(
        &self,
        _account: &str,
        database: &str,
        fingerprint: &str,
        selection: &werc_domain::favorites::FavoriteSelection,
        expected: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, werc_domain::favorites::FavoriteWindow)> {
        use werc_domain::favorites::*;
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "owned"));
        }
        if let Some(e) = expected {
            assert_eq!(e, "f".repeat(64));
        }
        let n = if selection.exact {
            selection
                .after
                .as_ref()
                .unwrap()
                .local_id
                .parse::<u32>()
                .unwrap()
        } else if selection.after.is_some() {
            1
        } else {
            2
        };
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-favorite".into();
        }
        let anchor = FavoriteAnchor {
            local_id: n.to_string(),
            updated: 1700000000,
        };
        let xml = format!(
            "<favitem type=\"1\"><title>自动化测试收藏</title><desc>{}</desc><datalist><dataitem datatype=\"8\"><datatitle>owned.txt</datatitle><cdn_datakey>{}</cdn_datakey><cdn_dataurl>{}</cdn_dataurl><fullsize>10</fullsize></dataitem></datalist></favitem>",
            self.body.lock().unwrap(),
            "a".repeat(32),
            "b".repeat(64)
        );
        Ok((
            "f".repeat(64),
            FavoriteWindow {
                next: (!selection.exact && n == 2).then(|| anchor.clone()),
                rows: vec![FavoriteRaw {
                    anchor,
                    tags: Some(vec!["关联标签".into()]),
                    server_id: "9223372036854775808".into(),
                    kind: 1,
                    source_user: Some("owned".into()),
                    conversation: Some("filehelper".into()),
                    xml: Some(xml),
                    content_status: "parsed".into(),
                }],
                counts: std::collections::BTreeMap::from([(1, 2)]),
                scanned: 1,
                read_only: true,
                readonly_shm: true,
            },
        ))
    }

    fn submit_quote(&self, s: QuoteSubmission<'_>) -> AppResult<SendReceipt> {
        assert!(s.source.valid());
        assert_eq!(
            s.source.sha256,
            hex::encode(Sha256::digest(s.source.text.as_bytes()))
        );
        let expected_body = match s.source.server_id.as_str() {
            "9007199254741997" => "quoted-literal:\nexact private text".to_owned(),
            "9007199254741999" => "quoted-literal:\nexact group text".to_owned(),
            _ => format!("owned-message-for-{}", s.submission.context.target_id),
        };
        assert_eq!(s.source.text, expected_body);
        self.quote_calls.fetch_add(1, Ordering::SeqCst);
        self.quote_source_ids.lock().unwrap().push(s.source.server_id.clone());
        if let Some((entered, resume)) = self.quote_submit_pause.lock().unwrap().take() {
            entered.send(()).unwrap();
            resume.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        }
        Ok(SendReceipt {
            outbox_id: s.submission.outbox_id.into(),
            attempt_id: s.submission.attempt_id.into(),
            context_digest: s.submission.context_digest.into(),
            evidence: SendEvidence::NativeQueued,
            provider_reference: Some("owned-quote-callback".into()),
        })
    }

    fn read_live_group_members(
        &self,
        _account: &str,
        database: &str,
        fingerprint: &str,
        selection: &werc_domain::group::LiveGroupSelection,
        expected: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, werc_domain::group::LiveGroupWindow)> {
        use werc_domain::group::*;
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-group".into();
        }
        if let Some(expected) = expected {
            assert_eq!(expected, "c".repeat(64));
        }
        let range = if let Some(after) = &selection.after {
            assert_eq!(after.row_id, "200");
            201..=201
        } else {
            1..=200
        };
        Ok((
            "c".repeat(64),
            LiveGroupWindow {
                cached_pins: Some(vec![LiveGroupPin{server_id:"18446744073709551615".into()}]),
                schema_version: 1,
                username: if self.invalid_group.load(Ordering::SeqCst) {
                    "wrong@chatroom".into()
                } else {
                    selection.username.clone()
                },
                owner_username: Some("owned-owner".into()),
                read_only: true,
                readonly_shm: true,
                has_more: selection.after.is_none(),
                rows: range
                    .map(|n| LiveGroupMember {
                        group_nickname:None,
                        anchor: LiveGroupAnchor {
                            row_id: n.to_string(),
                            member_id: Some(n.to_string()),
                            username: (n % 5 != 0).then(|| {
                                format!(
                                    "member-{n}-{}",
                                    selection.username.trim_end_matches("@chatroom")
                                )
                            }),
                        },
                        nickname: (n % 5 != 0).then(|| "owned-member-nickname".into()),
                        remark: None,
                    })
                    .collect(),
            },
        ))
    }

    fn read_live_contacts(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        after: Option<&LiveContactContinuation>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveContactWindow)> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-contacts".into();
        }
        let range = if let Some(after) = after {
            assert_eq!(after.source_fingerprint, "c".repeat(64));
            assert_eq!(after.anchor.row_id, "200");
            201..=201
        } else {
            1..=200
        };
        let friend_fixture = self.friend_fixture.load(Ordering::SeqCst);
        Ok((
            "c".repeat(64),
            LiveContactWindow {
                schema_version: 1,
                read_only: true,
                readonly_shm: true,
                has_more: after.is_none(),
                rows: range
                    .map(|n| LiveContact {
                        anchor: LiveContactAnchor {
                            row_id: n.to_string(),
                            username: match (friend_fixture, n) {
                                (true, 3) => "group@chatroom".into(),
                                (true, 4) => "owned".into(), // the active login
                                (true, 5) => "gh_sample".into(),
                                (true, 6) => "sample@openim".into(),
                                _ => format!("owned-contact-{n}"),
                            },
                        },
                        nickname: Some("自有昵称".into()),
                        remark: (friend_fixture && n == 1).then(|| "Owned remark".into()),
                        alias: None,
                        local_type: Some(i64::from(friend_fixture && matches!(n, 1 | 2 | 3 | 4 | 201))),
                        delete_flag: (friend_fixture && n == 2).then_some(1),
                        verify_flag: None,
                        contact_flag: None,
                        business_type: None,
            avatar_url: (n==201).then(||"https://wx.qlogo.cn/owned-avatar".into()),
            description: None,
                    })
                    .collect(),
            },
        ))
    }
    fn read_live_sessions(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        after: Option<&LiveSessionContinuation>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveSessionWindow)> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-sessions".into();
        }
        let range = if let Some(after) = after {
            assert_eq!(after.source_fingerprint, "c".repeat(64));
            assert_eq!(after.anchor.row_id, "2");
            1..=1
        } else {
            2..=201
        };
        Ok((
            "c".repeat(64),
            LiveSessionWindow {
                schema_version: 1,
                read_only: true,
                readonly_shm: true,
                has_more: after.is_none(),
                rows: range
                    .rev()
                    .map(|n| LiveSession {
                        anchor: LiveSessionAnchor {
                            row_id: n.to_string(),
                            sort_timestamp: 20,
                            username: format!("owned-session-{n}"),
                        },
                        summary: Some("自有摘要".into()),
                        summary_status: "text".into(),
                        last_timestamp: Some(19),
                        unread_count: Some(0),
                        is_hidden: Some(0),
                    })
                    .collect(),
            },
        ))
    }
    fn read_moment_media(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        selection: &MomentMediaSelection,
        cancel: &AtomicBool,
    ) -> AppResult<MomentMediaBytes> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-media".into();
        }
        if selection.slot != 0 {
            return Err(AppError::new("MOMENT_MEDIA_MISSING", "owned"));
        }
        Ok(MomentMediaBytes {
            moment: LiveMoment {
                anchor: selection.anchor.clone(), content_status: "parsed".into(),
                content: Some(MomentContent {
                    details: { let mut details=MomentDetails::default(); details.media=vec![MomentMediaInfo {
                        slot: 0, kind: Some(2), title: String::new(), description: String::new(),
                        width: None, height: None, duration: None, live_photo: false,
                    }]; details },
                    timestamp: 1700000000, text: "自动化测试：手动保存媒体".into(),
                    content_type: Some(1), media_count: 1, cached_like_count: 0, cached_comment_count: 0,
                }),
            },
            bytes: zeroize::Zeroizing::new(
                include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
            ),
            mime: "image/png",
            extension: "png",
            source_md5_matches: Some(true),
        })
    }
    fn read_live_moments(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        after: Option<&LiveMomentContinuation>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveMomentWindow)> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-moments".into();
        }
        let range = if let Some(after) = after {
            assert_eq!(after.source_fingerprint, "c".repeat(64));
            assert_eq!(after.anchor.row_id, "2");
            1..=1
        } else {
            2..=31
        };
        Ok((
            "c".repeat(64),
            LiveMomentWindow {
                schema_version: 1,
                read_only: true,
                readonly_shm: true,
                has_more: after.is_none(),
                rows: range
                    .rev()
                    .map(|n| LiveMoment {
                        anchor: LiveMomentAnchor {
                            row_id: n.to_string(),
                            id: (u64::MAX - 32 + n as u64).to_string(),
                            username: format!("owned-moment-{n}"),
                        },
                        content: Some(MomentContent {
                            details: if n == 31 && self.moment_media_enabled.load(Ordering::SeqCst)
                            {
                                let mut details = MomentDetails::default();
                                details.media = vec![MomentMediaInfo {
                                    slot: 0,
                                    kind: Some(2),
                                    title: String::new(),
                                    description: String::new(),
                                    width: None,
                                    height: None,
                                    duration: None,
                                    live_photo: false,
                                }];
                                details
                            } else {
                                Default::default()
                            },
                            timestamp: 1700000000,
                            text: "自有朋友圈".into(),
                            content_type: Some(1),
                            media_count: 1,
                            cached_like_count: 2,
                            cached_comment_count: 0,
                        }),
                        content_status: "parsed".into(),
                    })
                    .collect(),
            },
        ))
    }
    fn render_received_emoticon(
        &self,
        bytes: &[u8],
        cancel: &AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        if cancel.load(Ordering::SeqCst) || self.animation_error.load(Ordering::SeqCst) {
            return Err(AppError::new(
                "EMOTICON_CODEC_FAILED",
                "owned decode failure",
            ));
        }
        let animation = werc_domain::wxgf::parse(bytes)?;
        let mut raw = Vec::new();
        for n in [0x45525657u32, 1, 16, 16, 3] {
            raw.extend_from_slice(&n.to_le_bytes());
        }
        raw.resize(20 + 3 * 16 * 16 * 4, 0);
        for n in [0x45525657u32, 3] {
            raw.extend_from_slice(&n.to_le_bytes());
        }
        werc_domain::wxgf::encode_preview(std::io::Cursor::new(raw), &animation.info, &[30, 40, 50])
    }
    fn read_received_emoticon(
        &self,
        _: &str,
        _: &SendContext,
        _: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        cancel: &AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-emoticon".into();
        }
        if self.voice_missing.load(Ordering::SeqCst) {
            return Err(AppError::new("EMOTICON_CACHE_UNAVAILABLE", "owned"));
        }
        Ok(zeroize::Zeroizing::new(
            if self.body.lock().unwrap().contains("<emoticonmd5>") {
                owned_wxgf().to_vec()
            } else {
                owned_emoticon().to_vec()
            },
        ))
    }
    fn read_live_emoticon(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        _: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        cancel: &AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-emoticon".into();
        }
        if self.voice_missing.load(Ordering::SeqCst) {
            return Err(AppError::new("EMOTICON_CACHE_UNAVAILABLE", "owned"));
        }
        Ok(zeroize::Zeroizing::new(
            if self.body.lock().unwrap().contains("<emoticonmd5>") {
                owned_wxgf().to_vec()
            } else {
                owned_emoticon().to_vec()
            },
        ))
    }
    fn read_live_forward_file(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        _: &werc_domain::message_card::forward_file::ForwardFile,
        cancel: &AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-forward-file".into();
        }
        if self.voice_missing.load(Ordering::SeqCst) {
            return Err(AppError::new("FORWARD_FILE_CACHE_UNAVAILABLE", "owned"));
        }
        Ok(zeroize::Zeroizing::new(b"abc".to_vec()))
    }
    fn read_live_voice(
        &self,
        _: &str,
        database: &str,
        fingerprint: &str,
        anchor: &LiveVoiceAnchor,
        cancel: &AtomicBool,
    ) -> AppResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?
            .require(fingerprint)?;
        *self.last_voice.lock().unwrap() = Some((anchor.conversation().into(), database.into()));
        assert_eq!(anchor.server_id, "9007199254741242");
        assert_eq!(
            anchor.timestamp,
            1700000000
                + if self.spread_time.load(Ordering::SeqCst) {
                    250
                } else {
                    0
                }
        );
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-voice".into();
        }
        Ok(if self.voice_missing.load(Ordering::SeqCst) {
            None
        } else {
            Some(zeroize::Zeroizing::new(
                b"\x02#!SILK_V3\x01\x00\x03".to_vec(),
            ))
        })
    }
    fn asr_status(&self) -> AppResult<werc_domain::asr::AsrStatus> {
        Ok(werc_domain::asr::AsrStatus {
            available: true,
            models: vec!["owned".into()],
            engine: "owned-fixture",
        })
    }
    fn transcribe_audio(
        &self,
        wav: &[u8],
        model: &str,
        language: &str,
        cancel: &AtomicBool,
    ) -> AppResult<werc_domain::asr::Transcript> {
        assert_eq!(&wav[..4], b"RIFF");
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "owned"));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(werc_domain::asr::Transcript {
            model: model.into(),
            language: language.into(),
            text: "自动化测试".into(),
            segments: vec![werc_domain::asr::Segment {
                from_ms: 0,
                until_ms: 100,
                text: "自动化测试".into(),
            }],
        })
    }
    fn decode_received_voice(&self, _: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Ok(zeroize::Zeroizing::new(owned_voice_wav()))
    }

    fn download_original_image(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.download_received_image(context, descriptor)
    }
    fn download_received_image(
        &self,
        _: &SendContext,
        _: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Ok(zeroize::Zeroizing::new(
            include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
        ))
    }
    fn download_received_video(
        &self,
        _: &SendContext,
        _: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Ok(zeroize::Zeroizing::new(
            include_bytes!("../../../tests/fixtures/owned-send-video.mp4").to_vec(),
        ))
    }
    fn download_received_file(
        &self,
        _: &SendContext,
        _: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-download".into();
        }
        Ok(zeroize::Zeroizing::new(b"owned file".to_vec()))
    }

    fn live_source_fingerprint(&self, database: &str) -> AppResult<String> {
        werc_wechat_win::live_source::fingerprint(std::path::Path::new(database))
    }
    fn acknowledge_receive(&self, _: &str, epoch: &str, sequence: u64) -> AppResult<()> {
        assert_eq!(epoch, self.context.lock().unwrap().epoch);
        assert_eq!(sequence, self.quote_observation.lock().unwrap().as_ref().map_or(1, |(_, message)| message.sequence));
        Ok(())
    }
    fn poll_receive(&self, _: &str) -> AppResult<werc_domain::receive::ReceiveBatch> {
        if let Some((route, message)) = self.quote_observation.lock().unwrap().clone() {
            return Ok(werc_domain::receive::ReceiveBatch {
                route: Some(route), inline_voice: None, context: self.context.lock().unwrap().clone(),
                dropped: 0, message: Some(message),
            });
        }
        let body = self.body.lock().unwrap().clone();
        Ok(werc_domain::receive::ReceiveBatch {
            route: None,
            inline_voice: None,
            context: self.context.lock().unwrap().clone(),
            dropped: 0,
            message: if body.contains("<emoji ") || body.contains("<emoticonmd5>") {
                Some(werc_domain::receive::ReceivedText {
                    sequence: 1,
                    native_message_id: "9007199254741999".into(),
                    text: String::new(),
                    message_type: if body.contains("<emoticonmd5>") {
                        49
                    } else {
                        47
                    },
                    metadata: Some(body.clone()),
                })
            } else {
                None
            },
        })
    }
    fn validate_live_source(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn prepare_recipient(
        &self,
        _: &str,
        target: &str,
        sources: &[(String, String)],
    ) -> AppResult<PreparedRecipient> {
        assert!(!sources.is_empty());
        for (path, fingerprint) in sources {
            werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(path))?
                .require(fingerprint)?;
        }
        self.recipients.lock().unwrap().push(target.into());
        Ok(PreparedRecipient {
            target_id: target.into(),
            display_name: target.into(),
            expires_at: Some(chrono::Utc::now().timestamp() + 300),
        })
    }
    fn resolve_send_context(&self, _: &str, target: &str) -> AppResult<SendContext> {
        let mut context = self.context.lock().unwrap().clone();
        if self
            .recipients
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == target)
        {
            context.target_id = target.into();
            context.target_display_name = target.into();
        }
        Ok(context)
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        let key = WindowsWechatProvider.unprotect_key(c, k)?;
        let hook = {
            let mut pending = self.on_unprotect.lock().unwrap();
            if pending.as_ref().is_some_and(|(scope, _)| scope == c) {
                pending.take()
            } else {
                None
            }
        };
        if let Some((_, hook)) = hook { hook(); }
        Ok(key)
    }
    fn database_salt(&self, _: &str) -> AppResult<[u8; 16]> {
        Ok(*self.salt.lock().unwrap())
    }
    fn live_reader_fingerprint(&self) -> AppResult<String> {
        Ok(self.engine.lock().unwrap().clone())
    }
    fn read_live_calendar(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        fingerprint: &str,
        selection: &LiveCalendarSelection,
        cancel: &AtomicBool,
    ) -> AppResult<LiveCalendarWindow> {
        let mut counts = Vec::new();
        for day in &selection.days {
            let mut query = LiveMessageQuery::default();
            query.from_timestamp = Some(day.from_timestamp);
            query.until_timestamp = Some(day.until_timestamp);
            let mut before = None;
            let mut count = 0;
            loop {
                let window = self.read_live_conversation(
                    account,
                    database,
                    key,
                    fingerprint,
                    &LiveConversationSelection {
                        username: selection.username.clone(),
                        before,
                        include_anchor: false,
                        newer_context: false,
                        query: Some(query.clone()),
                    },
                    cancel,
                )?;
                count += window.rows.len() as u64;
                if !window.has_older {
                    break;
                }
                let last = window.rows.last().unwrap();
                before = Some(LiveAnchor {
                    local_id: last.local_id.clone(),
                    server_id: last.server_id.clone(),
                    timestamp: last.timestamp,
                });
            }
            counts.push(count);
        }
        Ok(LiveCalendarWindow {
            counts,
            read_only: true,
            readonly_shm: true,
        })
    }
    fn search_live_messages(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        fingerprint: &str,
        selection: &LiveSearchSelection,
        cancel: &AtomicBool,
    ) -> AppResult<LiveSearchWindow> {
        let names = ["filehelper", "owned-friend"];
        let mut result = LiveSearchWindow {
            rows: vec![],
            next: None,
            scanned_conversations: 0,
            read_only: true,
            readonly_shm: true,
        };
        for (index, username) in names.iter().enumerate() {
            if selection.after.as_ref().is_some_and(|p| {
                *username < p.username.as_str() || (*username == p.username && p.before.is_none())
            }) {
                continue;
            }
            result.scanned_conversations += 1;
            let before = selection
                .after
                .as_ref()
                .filter(|p| p.username == *username)
                .and_then(|p| p.before.clone());
            let window = self.read_live_conversation(
                account,
                database,
                key,
                fingerprint,
                &LiveConversationSelection {
                    username: (*username).into(),
                    before,
                    query: Some(selection.query.clone()),
                    include_anchor: false,
                    newer_context: false,
                },
                cancel,
            )?;
            let remaining = LIVE_SEARCH_MAX_ROWS - result.rows.len();
            let more = window.has_older || window.rows.len() > remaining;
            result
                .rows
                .extend(
                    window
                        .rows
                        .into_iter()
                        .take(remaining)
                        .map(|message| LiveSearchHit {
                            username: (*username).into(),
                            message,
                        }),
                );
            if more {
                let last = &result.rows.last().unwrap().message;
                result.next = Some(LiveSearchPosition {
                    username: (*username).into(),
                    before: Some(LiveAnchor {
                        timestamp: last.timestamp,
                        local_id: last.local_id.clone(),
                        server_id: last.server_id.clone(),
                    }),
                });
                break;
            }
            if result.rows.len() == LIVE_SEARCH_MAX_ROWS {
                if index + 1 < names.len() {
                    result.next = Some(LiveSearchPosition {
                        username: (*username).into(),
                        before: None,
                    });
                }
                break;
            }
        }
        Ok(result)
    }
    fn read_live_conversation(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        fingerprint: &str,
        selection: &LiveConversationSelection,
        cancel: &AtomicBool,
    ) -> AppResult<LiveWindow> {
        assert!(selection.valid());
        if *self.body.lock().unwrap() == "owned-conversation-missing-second"
            && std::path::Path::new(database).file_name().unwrap() == "message_1.db"
        {
            return Err(AppError::new(
                "LIVE_CONVERSATION_NOT_IN_SOURCE",
                "owned missing conversation",
            ));
        }
        let mut window = self.read_live_filehelper(
            account,
            database,
            key,
            fingerprint,
            if selection.query.is_some() || selection.include_anchor || selection.newer_context {
                None
            } else {
                selection.before.as_ref()
            },
            cancel,
        )?;
        if selection.query.is_some() || selection.include_anchor || selection.newer_context {
            while window.has_older {
                let row = window.rows.last().unwrap();
                let anchor = LiveAnchor {
                    timestamp: row.timestamp,
                    local_id: row.local_id.clone(),
                    server_id: row.server_id.clone(),
                };
                let next = self.read_live_filehelper(
                    account,
                    database,
                    key,
                    fingerprint,
                    Some(&anchor),
                    cancel,
                )?;
                window.has_older = next.has_older;
                window.rows.extend(next.rows);
            }
            if let Some(anchor) = &selection.before {
                assert!(window.rows.iter().any(|r| r.local_id == anchor.local_id
                    && r.timestamp == anchor.timestamp
                    && r.server_id == anchor.server_id));
                window.rows.retain(|r| {
                    let k = (r.timestamp, r.local_id.parse::<i64>().unwrap());
                    let b = anchor.key().unwrap();
                    if selection.newer_context {
                        k > b
                    } else if selection.include_anchor {
                        k <= b
                    } else {
                        k < b
                    }
                });
            }
        }
        if selection.username == "owned-empty-recipient" {
            window.rows.clear();
            window.has_older = false;
        }
        for row in &mut window.rows {
            if row.text.is_some() {
                row.text = Some(format!("owned-message-for-{}", selection.username));
            }
        }
        if let Some(query) = &selection.query {
            window.rows.retain(|r| query.matches(r));
        }
        if selection.newer_context {
            window.rows.reverse();
            window.rows.truncate(30);
            window.rows.reverse();
            window.has_older = false;
        } else if selection.query.is_some() || selection.include_anchor {
            window.has_older = window.rows.len() > 125;
            window.rows.truncate(125);
        }
        Ok(window)
    }
    fn read_live_filehelper(
        &self,
        _: &str,
        database: &str,
        key: &[u8],
        source_fingerprint: &str,
        before: Option<&LiveAnchor>,
        cancel: &AtomicBool,
    ) -> AppResult<LiveWindow> {
        assert_eq!(key, &[0x5a; 32]);
        let pin =
            werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(database))?;
        pin.require(source_fingerprint)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        while self.wait.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "owned"));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.change.load(Ordering::SeqCst) {
            self.context.lock().unwrap().epoch = "changed-during-read".into();
        }
        let upper = before.map(|a| a.key().unwrap().1 - 1).unwrap_or(250);
        let spread = self.spread_time.load(Ordering::SeqCst);
        let lower = (upper - if spread { 123 } else { 124 }).max(1);
        Ok(LiveWindow {
            schema_version: 1,
            read_only: true,
            readonly_shm: true,
            read_mode: "bounded_transaction_window".into(),
            has_older: lower > 1,
            rows: (lower..=upper)
                .rev()
                .map(|n| LiveMessage {
                    at_user_list: None,
                    local_id: n.to_string(),
                    sender_username: (!self.legacy_sender.load(Ordering::SeqCst))
                        .then(|| format!("owned-sender-{}", n % 3)),
                    server_id: Some((9007199254740992_i64 + n).to_string()),
                    timestamp: 1700000000 + if spread { n } else { 0 },
                    message_type: if self.voice.load(Ordering::SeqCst) {
                        "34"
                    } else if self.body.lock().unwrap().starts_with("<msg username=") {
                        "42"
                    } else if self.body.lock().unwrap().contains("<location ") {
                        "48"
                    } else if self.body.lock().unwrap().contains("<img ") {
                        "3"
                    } else if self.body.lock().unwrap().contains("<videomsg ") {
                        "43"
                    } else if self.body.lock().unwrap().contains("<emoji ") {
                        "47"
                    } else if self.body.lock().unwrap().starts_with("<msg") {
                        "25769803825"
                    } else {
                        "1"
                    }
                    .into(),
                    text: if self.body.lock().unwrap().starts_with("<msg") {
                        None
                    } else {
                        Some(self.body.lock().unwrap().clone())
                    },
                    content_status: if self.body.lock().unwrap().starts_with("<msg") {
                        "metadata"
                    } else if self.compressed_text.load(Ordering::SeqCst) {
                        "text_zstd"
                    } else {
                        "text"
                    }
                    .into(),
                    metadata: if self.body.lock().unwrap().starts_with("<msg") {
                        let body = self.body.lock().unwrap().clone();
                        Some(if self.sender_envelope.load(Ordering::SeqCst) {
                            format!("owned-sender-{}:\n<?xml version=\"1.0\"?>{body}", n % 3)
                        } else {
                            body
                        })
                    } else {
                        None
                    },
                })
                .collect(),
        })
    }
}

#[test]
fn multi_recovery_preserves_overlapping_ids_and_survives_reconnect_and_store_reopen() {
    let f = Fixture::new();
    f.reader.spread_time.store(true, Ordering::SeqCst);
    let second = f.database.with_file_name("message_1.db");
    std::fs::write(&second, vec![0; 4096]).unwrap();
    let paths = vec![
        f.database.to_str().unwrap().into(),
        second.to_str().unwrap().into(),
    ];
    f.app.receive_once_local(&f.account).unwrap();
    let gap = f.app.receive_cursor_local(&f.account).unwrap();
    let first = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-recovery-multi", true)
        .unwrap();
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &first.view_id, false)
            .is_err()
    );
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, "wrong-view", true)
            .is_err()
    );
    let saved = f
        .app
        .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
        .unwrap();
    assert_eq!(saved.rows, 124);
    assert_eq!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
            .unwrap(),
        saved
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 2);
    let read_all = |app: &Application| {
        let mut rows = Vec::new();
        for offset in [0, 60, 120] {
            let page = app
                .recovered_filehelper_page_local(&f.account, &saved.id, offset, 60)
                .unwrap();
            assert_eq!(page.source_count, 2);
            assert!(!page.gaps_closed);
            rows.extend(
                page.rows
                    .iter()
                    .zip(page.source_indices)
                    .map(|(r, s)| (s, r.local_id.clone(), r.text.clone())),
            );
        }
        rows
    };
    let before = read_all(&f.app);
    assert_eq!(before.len(), 124);
    assert_eq!(
        before
            .iter()
            .map(|(s, id, _)| (*s, id))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        124
    );
    assert_eq!(
        before
            .iter()
            .map(|(_, id, _)| id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        62
    );
    f.app.clear_live_filehelper_local().unwrap();
    f.reader.context.lock().unwrap().epoch = "new-epoch".into();
    let mut paths = paths;
    paths.reverse();
    let next = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-recovery-reconnected", true)
        .unwrap();
    assert_eq!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &next.view_id, true)
            .unwrap(),
        saved
    );
    f.app.clear_live_filehelper_local().unwrap();
    // New repository and disconnected provider use DPAPI only for saved pages.
    let app = Application::new(
        Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap()),
        Arc::new(WindowsWechatProvider),
    );
    assert_eq!(read_all(&app), before);
    assert_eq!(app.receive_cursor_local(&f.account).unwrap(), gap);
    assert_eq!(app.recovery_summaries_local(&f.account).unwrap().len(), 1);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert!(
        app.recovered_filehelper_page_local("other-account", &saved.id, 0, 60)
            .is_err()
    );
    assert!(
        app.recovered_filehelper_page_local(&f.account, &saved.id, 125, 60)
            .is_err()
    );
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    for secret in ["owned-live-text", "db_storage", "new-epoch"] {
        assert!(!disk.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}

#[test]
fn multi_recovery_rejects_replaced_sources_and_damaged_existing_records() {
    let f = Fixture::new();
    let paths = vec![f.database.to_str().unwrap().into()];
    let first = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-multi-save-failure", true)
        .unwrap();
    let db = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER owned_multi_failure BEFORE INSERT ON receive_recovery_windows BEGIN SELECT RAISE(ABORT,'owned'); END;").unwrap();
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
            .is_err()
    );
    assert!(
        f.app
            .recovery_summaries_local(&f.account)
            .unwrap()
            .is_empty()
    );
    db.execute_batch("DROP TRIGGER owned_multi_failure")
        .unwrap();
    let saved = f
        .app
        .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
        .unwrap();
    db.execute_batch("UPDATE receive_recovery_windows SET ciphertext=zeroblob(length(ciphertext))")
        .unwrap();
    assert!(
        f.app
            .recovered_filehelper_page_local(&f.account, &saved.id, 0, 60)
            .is_err()
    );
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
            .is_err()
    );
    std::fs::rename(&f.database, f.database.with_extension("old")).unwrap();
    std::fs::write(&f.database, vec![0; 4096]).unwrap();
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &first.view_id, true)
            .is_err()
    );
}

#[test]
fn source_continuation_reads_a_new_bounded_window_and_rejects_replay_or_page_token() {
    let f = Fixture::new();
    let mut page = f
        .app
        .prepare_live_filehelper_local(
            &f.account,
            f.database.to_str().unwrap(),
            "owned-continue-first",
            true,
        )
        .unwrap();
    assert!(
        f.app
            .continue_live_filehelper_local(
                &f.account,
                page.older.as_ref().unwrap(),
                "owned-invalid-page"
            )
            .is_err()
    );
    let mut ids: Vec<i64> = Vec::new();
    loop {
        ids.extend(page.rows.iter().map(|r| r.local_id.parse::<i64>().unwrap()));
        if let Some(cursor) = page.older {
            page = f
                .app
                .live_filehelper_page_local(&f.account, Some((&cursor, LiveDirection::Older)), 60)
                .unwrap();
        } else {
            break;
        }
    }
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 1);
    let token = page.continuation.unwrap();
    f.app.cancel_browse_local("owned-continue-cancel").unwrap();
    assert_eq!(
        f.app
            .continue_live_filehelper_local(&f.account, &token, "owned-continue-cancel")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 1);
    page = f
        .app
        .continue_live_filehelper_local(&f.account, &token, "owned-continue-second")
        .unwrap();
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 2);
    assert!(
        f.app
            .continue_live_filehelper_local(&f.account, &token, "owned-continue-replay")
            .is_err()
    );
    loop {
        ids.extend(page.rows.iter().map(|r| r.local_id.parse::<i64>().unwrap()));
        if let Some(cursor) = page.older {
            page = f
                .app
                .live_filehelper_page_local(&f.account, Some((&cursor, LiveDirection::Older)), 60)
                .unwrap();
        } else {
            break;
        }
    }
    assert!(page.continuation.is_none());
    assert_eq!(ids, (1..=250).rev().collect::<Vec<_>>());
    assert!(f.app.history_sources_local(&f.account).unwrap().is_empty());
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[test]
fn source_binding_is_idempotent_and_atomic_against_revision_or_other_root() {
    let f = Fixture::new();
    f.app
        .bind_live_source_local(&f.account, f.database.to_str().unwrap(), true)
        .unwrap();
    let revision = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse::<i64>()
        .unwrap();
    assert_eq!(
        f.repo
            .bind_current_source(&f.account, revision - 1, &"f".repeat(64))
            .err()
            .unwrap()
            .code,
        "CONNECTION_CHANGED"
    );
    assert_eq!(
        f.repo
            .bind_current_source(&f.account, revision, &"f".repeat(64))
            .err()
            .unwrap()
            .code,
        "SOURCE_CONFLICT"
    );
    assert!(f.repo.source_owner(&"f".repeat(64)).unwrap().is_none());
    assert!(f.app.history_sources_local(&f.account).unwrap().is_empty());
}
struct Fixture {
    directory: PathBuf,
    database: PathBuf,
    account: String,
    repo: Arc<SqliteRepository>,
    reader: Arc<Reader>,
    app: Arc<Application>,
    _cleanup: owned_directory::OwnedDirectory,
}

fn two_sources(f: &Fixture) -> Vec<String> {
    let second = f.database.with_file_name("message_1.db");
    std::fs::copy(&f.database, &second).unwrap();
    vec![
        f.database.to_str().unwrap().into(),
        second.to_str().unwrap().into(),
    ]
}
#[test]
fn multi_source_key_preparation_reuses_verified_keys_and_honors_early_cancel() {
    let f = Fixture::new();
    let paths = two_sources(&f);
    let revision = f.repo.key_revision(&f.account).unwrap();
    f.app.cancel_browse_local("owned-multi-key-cancel").unwrap();
    assert_eq!(
        f.app
            .prepare_live_filehelper_multi_with_keys_local(
                &f.account,
                &paths,
                0,
                "owned-multi-key-cancel",
                true
            )
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 0);
    let page = f
        .app
        .prepare_live_filehelper_multi_with_keys_local(
            &f.account,
            &paths,
            0,
            "owned-multi-key-prepare",
            true,
        )
        .unwrap();
    assert_eq!(page.source_count, 2);
    assert_eq!(f.repo.key_revision(&f.account).unwrap(), revision);
}
#[test]
fn multi_source_pages_and_continuations_preserve_all_tied_ids_without_alias_merging() {
    let f = Fixture::new();
    let paths = two_sources(&f);
    let mut page = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-multi-first", true)
        .unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
    let mut windows = 0;
    loop {
        assert_eq!(page.source_count, 2);
        assert!(!page.source_snapshot_atomic);
        for row in &page.rows {
            assert!(seen.insert((row.source.clone(), row.message.local_id.clone())));
            ordered.push((
                row.message.timestamp,
                row.source.clone(),
                row.message.local_id.parse::<i64>().unwrap(),
            ));
        }
        if let Some(older) = page.older {
            page = f
                .app
                .live_filehelper_multi_page_local(
                    &f.account,
                    Some((&older, LiveDirection::Older)),
                    60,
                )
                .unwrap();
        } else if let Some(continuation) = page.continuation {
            windows += 1;
            page = f
                .app
                .continue_live_filehelper_multi_local(
                    &f.account,
                    &continuation,
                    &format!("owned-multi-next-{windows}"),
                )
                .unwrap();
            assert!(
                f.app
                    .continue_live_filehelper_multi_local(
                        &f.account,
                        &continuation,
                        "owned-multi-replay"
                    )
                    .is_err()
            );
        } else {
            break;
        }
        assert!(windows < 10);
    }
    assert_eq!(seen.len(), 500);
    assert!(ordered.windows(2).all(|w| w[0] > w[1]));
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
    assert!(f.app.history_sources_local(&f.account).unwrap().is_empty());
}
#[test]
fn multi_source_rejects_duplicate_file_identity_before_read_and_invalidation_covers_every_source() {
    let f = Fixture::new();
    let mut paths = two_sources(&f);
    let alias = f.database.with_file_name("message_2.db");
    std::fs::hard_link(&f.database, &alias).unwrap();
    paths.push(alias.to_str().unwrap().into());
    assert_eq!(
        f.app
            .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-multi-duplicate", true)
            .err()
            .unwrap()
            .code,
        "LIVE_SOURCE_DUPLICATE"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 0);
    paths.pop();
    f.app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-multi-valid", true)
        .unwrap();
    let changed = std::path::Path::new(&paths[1]);
    let old = changed.with_file_name("old.db");
    std::fs::rename(changed, &old).unwrap();
    std::fs::copy(&old, changed).unwrap();
    assert!(
        f.app
            .live_filehelper_multi_page_local(&f.account, None, 60)
            .is_err()
    );
}
#[test]
fn single_and_multi_views_replace_each_other_and_cancellation_never_publishes_partial_merge() {
    let f = Fixture::new();
    let paths = two_sources(&f);
    f.prepare().unwrap();
    f.app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-multi-replaces", true)
        .unwrap();
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_err()
    );
    f.prepare().unwrap();
    assert!(
        f.app
            .live_filehelper_multi_page_local(&f.account, None, 60)
            .is_err()
    );
    f.reader.wait.store(true, Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let before = f.reader.reads.load(Ordering::SeqCst);
    let task = std::thread::spawn(move || {
        app.prepare_live_filehelper_multi_local(&account, &paths, "owned-multi-cancel", true)
    });
    let start = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == before {
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.app.cancel_browse_local("owned-multi-cancel").unwrap();
    assert_eq!(task.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
    assert!(
        f.app
            .live_filehelper_multi_page_local(&f.account, None, 60)
            .is_err()
    );
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_ok()
    );
}

#[test]
fn same_path_salt_and_bytes_after_file_replacement_invalidate_live_views_and_recovery_scope() {
    let f = Fixture::new();
    let first = f.prepare().unwrap();
    let saved = f
        .app
        .recover_live_filehelper_local(&f.account, &first.view_id, true)
        .unwrap();
    let older = f
        .app
        .live_filehelper_page_local(
            &f.account,
            Some((first.older.as_ref().unwrap(), LiveDirection::Older)),
            100,
        )
        .unwrap();
    let token = older.continuation.unwrap();
    let old = f.database.with_file_name("original.db");
    std::fs::rename(&f.database, &old).unwrap();
    std::fs::copy(&old, &f.database).unwrap();
    assert_eq!(
        std::fs::read(&old).unwrap(),
        std::fs::read(&f.database).unwrap()
    );
    assert!(
        f.app
            .recover_live_filehelper_local(&f.account, &first.view_id, true)
            .is_err()
    );
    assert!(
        f.app
            .continue_live_filehelper_local(&f.account, &token, "owned-replaced-continue")
            .is_err()
    );
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_err()
    );
    let next = f.prepare().unwrap();
    let next_saved = f
        .app
        .recover_live_filehelper_local(&f.account, &next.view_id, true)
        .unwrap();
    assert_ne!(saved.id, next_saved.id);
    assert_eq!(f.app.recovery_summaries_local(&f.account).unwrap().len(), 2);
    assert_eq!(
        f.app
            .recovered_filehelper_page_local(&f.account, &saved.id, 0, 1)
            .unwrap()
            .rows[0]
            .text
            .as_deref(),
        Some("owned-live-text")
    );
}

#[test]
fn legacy_saved_window_without_sender_field_reopens_and_keeps_its_digest() {
    let f = Fixture::new();
    f.reader.legacy_sender.store(true, Ordering::SeqCst);
    let p = f.prepare().unwrap();
    assert!(p.rows.iter().all(|r| r.sender_username.is_none()));
    let saved = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    let reopened = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(reopened, f.reader.clone());
    let page = app
        .recovered_filehelper_page_local(&f.account, &saved.id, 0, 60)
        .unwrap();
    assert_eq!(page.rows.len(), 60);
    assert!(page.rows.iter().all(|r| {
        r.sender_username.is_none()
            && !serde_json::to_string(r)
                .unwrap()
                .contains("sender_username")
    }));
    assert_eq!(page.summary, saved);
}

#[test]
fn recovery_is_encrypted_durable_idempotent_and_never_closes_stream_gaps() {
    let f = Fixture::new();
    assert_eq!(f.app.receive_once_local(&f.account).unwrap().gaps, 1);
    let gap_before = f.app.receive_cursor_local(&f.account).unwrap();
    let p = f.prepare().unwrap();
    let saved = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    assert_eq!(saved.rows, 125);
    assert_eq!(
        f.app
            .recover_live_filehelper_local(&f.account, &p.view_id, true)
            .unwrap(),
        saved
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 1);
    assert_eq!(f.repo.receive_cursor(&f.account).unwrap(), gap_before);
    assert!(
        f.repo
            .received_texts(&f.account, 0, 100)
            .unwrap()
            .is_empty()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let reopened = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(reopened, f.reader.clone());
    assert_eq!(
        app.recovery_summaries_local(&f.account).unwrap(),
        vec![saved.clone()]
    );
    f.app.clear_live_filehelper_local().unwrap();
    f.reader.context.lock().unwrap().epoch = "disconnected".into();
    let first = app
        .recovered_filehelper_page_local(&f.account, &saved.id, 0, 60)
        .unwrap();
    let second = app
        .recovered_filehelper_page_local(&f.account, &saved.id, 60, 60)
        .unwrap();
    let third = app
        .recovered_filehelper_page_local(&f.account, &saved.id, 120, 60)
        .unwrap();
    assert_eq!(
        (first.rows.len(), second.rows.len(), third.rows.len()),
        (60, 60, 5)
    );
    assert!(!first.gaps_closed && third.next_offset.is_none());
    assert_eq!(
        first.rows[0].sender_username.as_deref(),
        Some("owned-sender-1")
    );
    assert!(
        app.recovered_filehelper_page_local("other-account", &saved.id, 0, 60)
            .is_err()
    );
    assert!(
        app.recovered_filehelper_page_local(&f.account, &saved.id, 126, 60)
            .is_err()
    );
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    for secret in [
        "owned-live-text",
        "owned-sender-1",
        "9007199254741242",
        "db_storage",
        "owned-live-epoch",
    ] {
        assert!(!disk.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}

#[test]
fn recovery_preserves_changed_source_observations_instead_of_overwriting_ids() {
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    let original = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    *f.reader.body.lock().unwrap() = "owned revised text".into();
    let p = f.prepare().unwrap();
    let revised = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    assert_ne!(original.id, revised.id);
    assert_eq!(f.app.recovery_summaries_local(&f.account).unwrap().len(), 2);
    assert_eq!(
        f.app
            .recovered_filehelper_page_local(&f.account, &original.id, 0, 1)
            .unwrap()
            .rows[0]
            .text
            .as_deref(),
        Some("owned-live-text")
    );
    assert_eq!(
        f.app
            .recovered_filehelper_page_local(&f.account, &revised.id, 0, 1)
            .unwrap()
            .rows[0]
            .text
            .as_deref(),
        Some("owned revised text")
    );
}

#[test]
fn recovery_rejects_unconfirmed_stale_view_and_generation_changes() {
    for mode in 0..4 {
        let f = Fixture::new();
        let p = f.prepare().unwrap();
        assert_eq!(
            f.app
                .recover_live_filehelper_local(&f.account, &p.view_id, false)
                .err()
                .unwrap()
                .code,
            "CONSENT_REQUIRED"
        );
        assert!(
            f.app
                .recover_live_filehelper_local(&f.account, "other-view", true)
                .is_err()
        );
        match mode {
            0 => f.reader.context.lock().unwrap().epoch = "changed".into(),
            1 => *f.reader.salt.lock().unwrap() = [0x44; 16],
            2 => *f.reader.engine.lock().unwrap() = "f".repeat(64),
            _ => {
                f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
            }
        }
        assert!(
            f.app
                .recover_live_filehelper_local(&f.account, &p.view_id, true)
                .is_err()
        );
        assert!(
            f.app
                .recovery_summaries_local(&f.account)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn recovery_write_failure_rolls_back_and_tampering_cannot_be_read() {
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    let db = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER owned_recovery_failure BEFORE INSERT ON receive_recovery_windows BEGIN SELECT RAISE(ABORT,'owned'); END;").unwrap();
    assert!(
        f.app
            .recover_live_filehelper_local(&f.account, &p.view_id, true)
            .is_err()
    );
    assert!(
        f.app
            .recovery_summaries_local(&f.account)
            .unwrap()
            .is_empty()
    );
    db.execute_batch("DROP TRIGGER owned_recovery_failure")
        .unwrap();
    let s = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    db.execute_batch("UPDATE receive_recovery_windows SET ciphertext=zeroblob(length(ciphertext))")
        .unwrap();
    assert_eq!(
        f.app
            .recovered_filehelper_page_local(&f.account, &s.id, 0, 60)
            .err()
            .unwrap()
            .code,
        "RECOVERY_CORRUPTED"
    );
    db.execute_batch("UPDATE receive_recovery_windows SET summary=json_set(summary,'$.rows',2)")
        .unwrap();
    assert_eq!(
        f.app
            .recovery_summaries_local(&f.account)
            .err()
            .unwrap()
            .code,
        "RECOVERY_CORRUPTED"
    );
}

#[test]
fn recovery_store_rechecks_connection_key_and_cursor_inside_transaction() {
    use werc_domain::recovery::RecoveryCommit;
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    let s = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    let record = f.repo.recovery(&f.account, &s.id).unwrap();
    let rev = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse::<i64>()
        .unwrap();
    let key = f.repo.key_revision(&f.account).unwrap();
    for (revision, key_revision) in [(rev - 1, key), (rev, key - 1)] {
        assert_eq!(
            f.repo
                .commit_recovery(&RecoveryCommit {
                    account_id: &f.account,
                    connection_revision: revision,
                    key_revision,
                    expected_receive: None,
                    record: &record
                })
                .err()
                .unwrap()
                .code,
            "RECOVERY_CONTEXT_CHANGED"
        );
    }
    let changed = werc_domain::receive::ReceiveCursor {
        proof: "a".repeat(64),
        last_id: None,
        epoch: "b".repeat(64),
        sequence: 0,
        dropped: 0,
        gaps: 1,
    };
    assert_eq!(
        f.repo
            .commit_recovery(&RecoveryCommit {
                account_id: &f.account,
                connection_revision: rev,
                key_revision: key,
                expected_receive: Some(&changed),
                record: &record
            })
            .err()
            .unwrap()
            .code,
        "RECOVERY_CONTEXT_CHANGED"
    );
}

#[test]
fn recovery_quota_is_bounded_and_repeated_save_does_not_consume_another_slot() {
    use werc_domain::recovery::RecoveryCommit;
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    let s = f
        .app
        .recover_live_filehelper_local(&f.account, &p.view_id, true)
        .unwrap();
    let mut record = f.repo.recovery(&f.account, &s.id).unwrap();
    let rev = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse::<i64>()
        .unwrap();
    let key = f.repo.key_revision(&f.account).unwrap();
    for n in 0..127 {
        record.summary.id = format!("{n:064x}");
        f.repo
            .commit_recovery(&RecoveryCommit {
                account_id: &f.account,
                connection_revision: rev,
                key_revision: key,
                expected_receive: None,
                record: &record,
            })
            .unwrap();
    }
    assert_eq!(
        f.app
            .recover_live_filehelper_local(&f.account, &p.view_id, true)
            .unwrap(),
        s
    );
    record.summary.id = "f".repeat(64);
    assert_eq!(
        f.repo
            .commit_recovery(&RecoveryCommit {
                account_id: &f.account,
                connection_revision: rev,
                key_revision: key,
                expected_receive: None,
                record: &record
            })
            .err()
            .unwrap()
            .code,
        "QUOTA_EXCEEDED"
    );
    assert_eq!(f.repo.recovery_summaries(&f.account).unwrap().len(), 128);
}
impl Fixture {
    fn new() -> Self {
        let cleanup =
            owned_directory::OwnedDirectory::new("werc-owned-live-app");
        let directory=cleanup.0.clone();
        let root = directory.join("account");
        let folder = root.join("db_storage/message");
        std::fs::create_dir_all(&folder).unwrap();
        let database = folder.join("message_0.db");
        std::fs::write(&database, vec![0; 4096]).unwrap();
        let repo = Arc::new(SqliteRepository::open(&directory.join("state.sqlite3")).unwrap());
        let canonical = root
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        let fp = hex::encode(Sha256::digest(
            format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
        ));
        let account = repo.setup_account_for_source("owned", &fp).unwrap().id;
        repo.save_binding(&ClientBinding {
            account_id: account.clone(),
            installation: ClientInstallation {
                id: "owned".into(),
                executable_path: "owned-no-executable".into(),
                version: "fixture".into(),
                architecture: "x86_64".into(),
                product_name: "fixture".into(),
                sha256: "1".repeat(64),
                process_ids: vec![],
                sources: vec![],
                compatibility: "fixture".into(),
                reason: "fixture".into(),
            },
            bound_at: "fixture".into(),
            checked_at: "fixture".into(),
            validation: "fixture".into(),
        })
        .unwrap();
        let reader = Arc::new(Reader {
            friend_fixture: AtomicBool::new(false),
            auto_sources: Mutex::new(Vec::new()),
            context: Mutex::new(SendContext {
                sender_id: "owned".into(),
                target_id: "filehelper".into(),
                target_display_name: "owned".into(),
                identity_verified: true,
                target_verified: true,
                epoch: "owned-live-epoch".into(),
                installation_sha256: "1".repeat(64),
                adapter_sha256: "2".repeat(64),
                execution_mode: SendExecutionMode::Synthetic,
            }),
            engine: Mutex::new("3".repeat(64)),
            salt: Mutex::new([0x42; 16]),
            reads: AtomicUsize::new(0),
            change: AtomicBool::new(false),
            wait: AtomicBool::new(false),
            body: Mutex::new("owned-live-text".into()),
            spread_time: AtomicBool::new(false),
            voice: AtomicBool::new(false),
            voice_missing: AtomicBool::new(false),
            legacy_sender: AtomicBool::new(false),
            invalid_group: AtomicBool::new(false),
            animation_error: AtomicBool::new(false),
            last_voice: Mutex::new(None),
            recipients: Mutex::new(Vec::new()),
            quote_calls: AtomicUsize::new(0),
            quote_source_ids: Mutex::new(Vec::new()),
            quote_submit_pause: Mutex::new(None),
            quote_observation: Mutex::new(None),
            compressed_text: AtomicBool::new(false),
            sender_envelope: AtomicBool::new(false),
            moment_media_enabled: AtomicBool::new(false),
            on_unprotect: Mutex::new(None),
        });
        let app = Arc::new(Application::new(repo.clone(), reader.clone()));
        app.import_key_local(&account, "5a".repeat(32)).unwrap();
        Self {
            directory,
            database,
            account,
            repo,
            reader,
            app,
            _cleanup:cleanup,
        }
    }
    fn prepare(&self) -> AppResult<werc_application::live_view::LivePage> {
        self.app.prepare_live_filehelper_local(
            &self.account,
            self.database.to_str().unwrap(),
            "owned-live-request-0001",
            true,
        )
    }
}

#[test]
fn protected_live_pages_do_not_import_or_submit_or_requery_the_source() {
    let f = Fixture::new();
    let first = f.prepare().unwrap();
    assert_eq!(first.rows.len(), 60);
    let second = f
        .app
        .live_filehelper_page_local(
            &f.account,
            Some((first.older.as_ref().unwrap(), LiveDirection::Older)),
            60,
        )
        .unwrap();
    assert_eq!(second.rows[0].local_id, "190");
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 1);
    assert!(f.app.history_sources_local(&f.account).unwrap().is_empty());
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(!disk.windows(15).any(|s| s == b"owned-live-text"));
    f.app.clear_live_filehelper_local().unwrap();
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_err()
    );
}
#[test]
fn live_pages_invalidate_on_login_engine_key_revision_and_source_generation() {
    for mode in 0..4 {
        let f = Fixture::new();
        f.prepare().unwrap();
        match mode {
            0 => f.reader.context.lock().unwrap().epoch = "changed".into(),
            1 => *f.reader.engine.lock().unwrap() = "4".repeat(64),
            2 => {
                f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
            }
            _ => *f.reader.salt.lock().unwrap() = [0x43; 16],
        }
        assert!(
            f.app
                .live_filehelper_page_local(&f.account, None, 60)
                .is_err()
        );
    }
}
#[test]
fn explicit_consent_source_ownership_and_post_read_context_are_required() {
    let f = Fixture::new();
    assert_eq!(
        f.app
            .prepare_live_filehelper_local(
                &f.account,
                f.database.to_str().unwrap(),
                "owned-live-request-0001",
                false
            )
            .err()
            .unwrap()
            .code,
        "CONSENT_REQUIRED"
    );
    let other = f.directory.join("other.db");
    std::fs::write(&other, vec![0; 4096]).unwrap();
    assert_eq!(
        f.app
            .prepare_live_filehelper_local(
                &f.account,
                other.to_str().unwrap(),
                "owned-live-request-0001",
                true
            )
            .err()
            .unwrap()
            .code,
        "SOURCE_CONFLICT"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 0);
    f.reader.change.store(true, Ordering::SeqCst);
    assert_eq!(f.prepare().err().unwrap().code, "CONNECTION_CHANGED");
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_err()
    );
}
#[test]
fn shares_existing_cancellation_before_and_during_read_without_publishing() {
    let f = Fixture::new();
    f.app
        .cancel_browse_local("owned-live-request-0001")
        .unwrap();
    assert_eq!(f.prepare().err().unwrap().code, "QUERY_CANCELED");
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 0);
    f.reader.wait.store(true, Ordering::SeqCst);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| f.prepare());
        let started = std::time::Instant::now();
        while f.reader.reads.load(Ordering::SeqCst) == 0 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        f.app
            .cancel_browse_local("owned-live-request-0001")
            .unwrap();
        assert_eq!(handle.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
    });
    assert!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .is_err()
    );
}

#[test]
fn live_file_download_binds_view_and_rechecks_identity_without_importing_or_sending() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = format!(
        "<msg><appmsg><title>owned.txt</title><type>6</type><appattach><totallen>10</totallen><cdnattachurl>aabbccdd</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
        "ab".repeat(16)
    );
    let page = f
        .app
        .prepare_live_filehelper_local(
            &f.account,
            f.database.to_str().unwrap(),
            "owned-file-live-0001",
            true,
        )
        .unwrap();
    let id = &page.rows[0].local_id;
    assert!(
        f.app
            .download_live_file_local(&f.account, "stale", id)
            .is_err()
    );
    assert!(
        f.app
            .download_live_file_local(&f.account, &page.view_id, "missing")
            .is_err()
    );
    let info = f
        .app
        .live_file_info_local(&f.account, &page.view_id, id)
        .unwrap();
    assert_eq!(info.file_name, "owned.txt");
    let file = f
        .app
        .download_live_file_local(&f.account, &page.view_id, id)
        .unwrap();
    assert_eq!(&*file.bytes, b"owned file");
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
    assert!(f.app.history_sources_local(&f.account).unwrap().is_empty());
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .download_live_file_local(&f.account, &page.view_id, id)
            .is_err()
    );
}

fn owned_voice_wav() -> Vec<u8> {
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&676u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&640u32.to_le_bytes());
    wav.extend_from_slice(&[0x35; 640]);
    wav
}
#[test]
fn live_voice_is_encrypted_and_survives_closed_store_reopen_without_sending_or_closing_gap() {
    let f = Fixture::new();
    f.reader.voice.store(true, Ordering::SeqCst);
    f.app.receive_once_local(&f.account).unwrap();
    let gap = f.repo.receive_cursor(&f.account).unwrap();
    let page = f.prepare().unwrap();
    let id = &page.rows[0].local_id;
    let asset = f
        .app
        .save_live_voice_local(&f.account, &page.view_id, id, "owned-voice-save-0001")
        .unwrap();
    assert_eq!(
        f.app
            .audio_preview_local(&f.account, &asset.id)
            .unwrap()
            .as_slice(),
        owned_voice_wav()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert_eq!(f.repo.receive_cursor(&f.account).unwrap(), gap);
    let fpath = f.directory.join("state.sqlite3");
    let account = f.account.clone();
    let reader = f.reader.clone();
    let _reopen_directory=f._cleanup.clone();
    drop(f);
    reader.context.lock().unwrap().epoch = "disconnected".into();
    let repo = Arc::new(SqliteRepository::open(&fpath).unwrap());
    let app = Application::new(repo.clone(), reader);
    assert_eq!(
        app.audio_preview_local(&account, &asset.id)
            .unwrap()
            .as_slice(),
        owned_voice_wav()
    );
    assert!(app.audio_preview_local("other-account", &asset.id).is_err());
    assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
    assert!(repo.sends(&account).unwrap().is_empty());
    let disk = std::fs::read(&fpath).unwrap();
    assert!(!disk.windows(64).any(|s| s == [0x35; 64]));
}
#[test]
fn live_voice_rejects_wrong_rows_stale_views_missing_bytes_identity_change_and_cancel() {
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    assert!(
        f.app
            .save_live_voice_local(
                &f.account,
                &p.view_id,
                &p.rows[0].local_id,
                "owned-voice-save-0001"
            )
            .is_err()
    );
    f.reader.voice.store(true, Ordering::SeqCst);
    let p = f.prepare().unwrap();
    let id = &p.rows[0].local_id;
    for (view, row) in [("stale", id.as_str()), (p.view_id.as_str(), "missing")] {
        assert!(
            f.app
                .save_live_voice_local(&f.account, view, row, "owned-voice-save-0002")
                .is_err()
        );
    }
    f.reader.voice_missing.store(true, Ordering::SeqCst);
    assert_eq!(
        f.app
            .save_live_voice_local(&f.account, &p.view_id, id, "owned-voice-save-0003")
            .unwrap_err()
            .code,
        "LIVE_VOICE_NOT_CACHED"
    );
    f.reader.voice_missing.store(false, Ordering::SeqCst);
    f.app.cancel_browse_local("owned-voice-save-0004").unwrap();
    assert_eq!(
        f.app
            .save_live_voice_local(&f.account, &p.view_id, id, "owned-voice-save-0004")
            .unwrap_err()
            .code,
        "QUERY_CANCELED"
    );
    f.reader.wait.store(true, Ordering::SeqCst);
    let before = f.reader.reads.load(Ordering::SeqCst);
    std::thread::scope(|s| {
        let call = s.spawn(|| {
            f.app
                .save_live_voice_local(&f.account, &p.view_id, id, "owned-voice-save-0005")
        });
        let started = std::time::Instant::now();
        while f.reader.reads.load(Ordering::SeqCst) == before {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        f.app.cancel_browse_local("owned-voice-save-0005").unwrap();
        assert_eq!(call.join().unwrap().unwrap_err().code, "QUERY_CANCELED");
    });
    f.reader.wait.store(false, Ordering::SeqCst);
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .save_live_voice_local(&f.account, &p.view_id, id, "owned-voice-save-0006")
            .is_err()
    );
    assert!(f.app.audio_assets_local(&f.account).unwrap().is_empty());
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[test]
fn live_voice_transaction_rejects_late_connection_and_key_revisions() {
    for changed_key in [false, true] {
        let f = Fixture::new();
        let a = f
            .app
            .upload_audio_local(&f.account, owned_voice_wav())
            .unwrap();
        let p = f.repo.asset(&f.account, &a.id).unwrap();
        f.app.delete_audio_local(&f.account, &a.id).unwrap();
        let revision = f
            .repo
            .current_connection()
            .unwrap()
            .revision
            .parse::<i64>()
            .unwrap();
        let key = f.repo.key_revision(&f.account).unwrap();
        if changed_key {
            f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
        }
        assert!(
            f.repo
                .save_live_audio(&p, if changed_key { revision } else { revision + 1 }, key)
                .is_err()
        );
        assert!(f.app.audio_assets_local(&f.account).unwrap().is_empty());
    }
}

#[test]
fn contacts_page_cursor_context_and_cancellation_do_not_persist_plaintext() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let key_revision = f.repo.key_revision(&f.account).unwrap();
    let first = f
        .app
        .live_contacts_local(&f.account, &view.view_id, None, "owned-contact-page-0001")
        .unwrap();
    assert_eq!(first.rows.len(), 200);
    assert!(!first.complete_address_book);
    assert!(!first.cross_page_snapshot);
    let next = first.next.as_deref().unwrap();
    let last = f
        .app
        .live_contacts_local(
            &f.account,
            &view.view_id,
            Some(next),
            "owned-contact-page-0002",
        )
        .unwrap();
    assert_eq!(last.rows.len(), 1);
    assert!(last.next.is_none());
    let before = f.reader.reads.load(Ordering::SeqCst);
    for (account, id, cursor) in [
        (&*f.account, "wrong", None),
        ("other", &*view.view_id, None),
        (&*f.account, &*view.view_id, Some("bad-token")),
    ] {
        assert!(
            f.app
                .live_contacts_local(account, id, cursor, "owned-contact-invalid")
                .is_err()
        );
    }
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), before);
    f.app.cancel_browse_local("owned-contact-canceled").unwrap();
    assert_eq!(
        f.app
            .live_contacts_local(&f.account, &view.view_id, None, "owned-contact-canceled")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.repo.key_revision(&f.account).unwrap(), key_revision);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let bytes = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(b"owned-contact-".len())
            .any(|w| w == b"owned-contact-")
    );
    let replacement = f.prepare().unwrap();
    assert!(
        f.app
            .live_contacts_local(
                &f.account,
                &replacement.view_id,
                Some(next),
                "owned-contact-stale"
            )
            .is_err()
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .live_contacts_local(
                &f.account,
                &replacement.view_id,
                None,
                "owned-contact-context"
            )
            .is_err()
    );
}

#[tokio::test]
async fn group_names_without_view_requires_explicit_scope_and_matching_account() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    f.reader.auto_sources.lock().unwrap().push(f.database.to_str().unwrap().into());
    let issue = |scope: &str| {
        f.app.issue_token_local("owned test", vec![f.account.clone()], vec![scope.into()], 1).unwrap()
    };
    let token = issue("group-names:read");
    let old = issue("contacts:read");
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().uri(format!("/api/v1/contacts/group-names?account_id={account}"))
            .header("Authorization", format!("Bearer {secret}")).body(Body::empty()).unwrap()
    };
    for (secret, account) in [(old.secret.as_str(), f.account.as_str()), (token.secret.as_str(), "other")] {
        assert_eq!(router.clone().oneshot(request(secret, account)).await.unwrap().status(), 403);
    }
    let response = router.oneshot(request(&token.secret, &f.account)).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({"account_id": f.account, "data": {"owned@chatroom": "Owned group"}}));
}

#[tokio::test]
async fn directory_friends_reads_all_pages_and_excludes_nonfriends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    f.reader.friend_fixture.store(true, Ordering::SeqCst);
    f.reader.auto_sources.lock().unwrap().push(f.database.to_str().unwrap().into());
    let issue = |scope: &str| {
        f.app.issue_token_local("owned test", vec![f.account.clone()], vec![scope.into()], 1).unwrap()
    };
    let token = issue("friends:read");
    let old = issue("contacts:read");
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().uri(format!("/api/v1/contacts/friends?account_id={account}"))
            .header("Authorization", format!("Bearer {secret}")).body(Body::empty()).unwrap()
    };
    for (secret, account) in [(old.secret.as_str(), f.account.as_str()), (token.secret.as_str(), "other")] {
        assert_eq!(router.clone().oneshot(request(secret, account)).await.unwrap().status(), 403);
    }
    let response = router.oneshot(request(&token.secret, &f.account)).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!({"account_id": f.account, "data_mode": "local_friend_directory", "data": [
        {"username": "owned-contact-1", "nickname": "自有昵称", "remark": "Owned remark"},
        {"username": "owned-contact-201", "nickname": "自有昵称", "remark": null}
    ]}));
}

#[tokio::test]
async fn categorized_directory_requires_scope_and_preserves_local_categories() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    f.reader.friend_fixture.store(true, Ordering::SeqCst);
    f.reader.auto_sources.lock().unwrap().push(f.database.to_str().unwrap().into());
    let issue = |scope: &str| {
        f.app.issue_token_local("owned test", vec![f.account.clone()], vec![scope.into()], 1).unwrap()
    };
    let token = issue("directory:read");
    let old = issue("friends:read");
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().uri(format!("/api/v1/contacts/directory?account_id={account}"))
            .header("Authorization", format!("Bearer {secret}")).body(Body::empty()).unwrap()
    };
    for (secret, account) in [(old.secret.as_str(), f.account.as_str()), (token.secret.as_str(), "other")] {
        assert_eq!(router.clone().oneshot(request(secret, account)).await.unwrap().status(), 403);
    }
    let response = router.oneshot(request(&token.secret, &f.account)).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data_mode"], "local_categorized_directory");
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 200);
    for (name, category) in [("owned-contact-201", "friend"), ("owned-contact-2", "deleted"),
        ("group@chatroom", "group"), ("gh_sample", "service"), ("sample@openim", "enterprise")] {
        assert_eq!(rows.iter().find(|r| r["username"] == name).unwrap()["category"], category);
    }
    assert!(!rows.iter().any(|r| r["username"] == "owned"));
}

#[tokio::test]
async fn contacts_http_checks_scope_range_and_revocation_after_read() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned contacts",
            vec![f.account.clone()],
            vec!["contacts:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/contacts/live")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"view_id":view.view_id,"cursor":null,"request_id":"owned-contact-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 200);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other contact caller",
            vec![f.account.clone()],
            vec!["contacts:read".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/contacts/live/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-contact-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-contact-"));
}

#[test]
fn sessions_page_cursor_context_and_cancellation_do_not_persist_plaintext() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let key_revision = f.repo.key_revision(&f.account).unwrap();
    let first = f
        .app
        .live_sessions_local(&f.account, &view.view_id, None, "owned-session-page-0001")
        .unwrap();
    assert_eq!(first.rows.len(), 200);
    assert!(!first.complete_session_list);
    assert!(!first.cross_page_snapshot);
    let next = first.next.as_deref().unwrap();
    let last = f
        .app
        .live_sessions_local(
            &f.account,
            &view.view_id,
            Some(next),
            "owned-session-page-0002",
        )
        .unwrap();
    assert_eq!(last.rows.len(), 1);
    assert!(last.next.is_none());
    let before = f.reader.reads.load(Ordering::SeqCst);
    for (account, id, cursor) in [
        (&*f.account, "wrong", None),
        ("other", &*view.view_id, None),
        (&*f.account, &*view.view_id, Some("bad-token")),
    ] {
        assert!(
            f.app
                .live_sessions_local(account, id, cursor, "owned-session-invalid")
                .is_err()
        );
    }
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), before);
    f.app.cancel_browse_local("owned-session-canceled").unwrap();
    assert_eq!(
        f.app
            .live_sessions_local(&f.account, &view.view_id, None, "owned-session-canceled")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.repo.key_revision(&f.account).unwrap(), key_revision);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let bytes = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(b"owned-session-".len())
            .any(|w| w == b"owned-session-")
    );
    let replacement = f.prepare().unwrap();
    assert!(
        f.app
            .live_sessions_local(
                &f.account,
                &replacement.view_id,
                Some(next),
                "owned-session-stale"
            )
            .is_err()
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .live_sessions_local(
                &f.account,
                &replacement.view_id,
                None,
                "owned-session-context"
            )
            .is_err()
    );
}

#[tokio::test]
async fn sessions_http_checks_scope_range_and_revocation_after_read() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned sessions",
            vec![f.account.clone()],
            vec!["sessions:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/sessions/live")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"view_id":view.view_id,"cursor":null,"request_id":"owned-session-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 200);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other session caller",
            vec![f.account.clone()],
            vec!["sessions:read".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions/live/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-session-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-session-"));
}

#[test]
fn conversation_views_bind_username_source_and_do_not_enable_send_or_recovery() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let open = |name: &str| C::Open {
        parent_view_id: parent.view_id.clone(),
        username: name.into(),
    };
    let first = f
        .app
        .live_conversation_local(&f.account, open("owned-A"), "owned-conversation-open")
        .unwrap()
        .unwrap();
    assert_eq!(first.messages.rows.len(), 60);
    assert!(!first.sending_enabled);
    assert_eq!(
        first.messages.rows[0].text.as_deref(),
        Some("owned-message-for-owned-A")
    );
    let page = |view: &str, cursor: &str| C::Page {
        view_id: view.into(),
        cursor: cursor.into(),
        direction: LiveDirection::Older,
    };
    let next = f
        .app
        .live_conversation_local(
            &f.account,
            page(
                &first.messages.view_id,
                first.messages.older.as_deref().unwrap(),
            ),
            "owned-conversation-page",
        )
        .unwrap()
        .unwrap();
    assert_eq!(next.messages.rows.len(), 60);
    let end = f
        .app
        .live_conversation_local(
            &f.account,
            page(
                &next.messages.view_id,
                next.messages.older.as_deref().unwrap(),
            ),
            "owned-conversation-end",
        )
        .unwrap()
        .unwrap();
    let continued = f
        .app
        .live_conversation_local(
            &f.account,
            C::Continue {
                view_id: end.messages.view_id,
                cursor: end.messages.continuation.unwrap(),
            },
            "owned-conversation-continue",
        )
        .unwrap()
        .unwrap();
    assert_eq!(continued.messages.rows[0].local_id, "125");
    assert!(
        f.app
            .live_conversation_local(
                &f.account,
                page(
                    &continued.messages.view_id,
                    parent.older.as_deref().unwrap()
                ),
                "owned-conversation-mixed"
            )
            .is_err()
    );
    let second = f
        .app
        .live_conversation_local(&f.account, open("owned-B"), "owned-conversation-other")
        .unwrap()
        .unwrap();
    assert_eq!(
        second.messages.rows[0].text.as_deref(),
        Some("owned-message-for-owned-B")
    );
    assert!(
        f.app
            .live_conversation_local(
                &f.account,
                page(
                    &second.messages.view_id,
                    first.messages.older.as_deref().unwrap()
                ),
                "owned-conversation-replay"
            )
            .is_err()
    );
    assert!(
        f.app
            .recover_live_filehelper_local(&f.account, &second.messages.view_id, true)
            .is_err()
    );
    assert!(
        f.app
            .live_conversation_local(
                "other",
                page(
                    &second.messages.view_id,
                    second.messages.older.as_deref().unwrap()
                ),
                "owned-conversation-wrong"
            )
            .is_err()
    );
    f.app
        .live_conversation_local(
            &f.account,
            C::Clear {
                view_id: second.messages.view_id.clone(),
            },
            "owned-conversation-clear",
        )
        .unwrap();
    assert!(
        f.app
            .live_conversation_local(
                &f.account,
                page(
                    &second.messages.view_id,
                    second.messages.older.as_deref().unwrap()
                ),
                "owned-conversation-cleared"
            )
            .is_err()
    );
    f.app
        .cancel_browse_local("owned-conversation-canceled")
        .unwrap();
    assert_eq!(
        f.app
            .live_conversation_local(&f.account, open("owned-A"), "owned-conversation-canceled")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .live_conversation_local(&f.account, open("owned-A"), "owned-conversation-changed")
            .is_err()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !disk
            .windows(b"owned-message-for".len())
            .any(|w| w == b"owned-message-for")
    );
}

#[tokio::test]
async fn conversation_http_checks_scope_range_and_revocation_after_read() {
    conversation_http_authorization_case(false).await;
}
#[tokio::test]
async fn multi_conversation_http_checks_scope_range_cancel_isolation_and_revocation() {
    conversation_http_authorization_case(true).await;
}
async fn conversation_http_authorization_case(multi: bool) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view_id = if multi {
        f.app
            .prepare_live_filehelper_multi_local(
                &f.account,
                &two_sources(&f),
                "owned-multi-http-prepare",
                true,
            )
            .unwrap()
            .view_id
    } else {
        f.prepare().unwrap().view_id
    };
    let token = f
        .app
        .issue_token_local(
            "owned sessions",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["sessions:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri(if multi {"/api/v1/messages/live/conversation/multi"} else {"/api/v1/messages/live/conversation"})
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"command":{"action":"open","parent_view_id":view_id,"username":"owned-A"},"request_id":"owned-session-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["messages"]["rows"].as_array().unwrap().len(), 60);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other session caller",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/messages/live/conversation/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-session-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-message-for"));
}

#[test]
fn clearing_parent_during_conversation_read_discards_late_result() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let task = std::thread::spawn(move || {
        app.live_conversation_local(
            &account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-conversation-clear-race",
        )
    });
    let start = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.app.clear_live_filehelper_local().unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    assert_eq!(task.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
}

#[test]
fn clear_without_owned_view_cannot_invalidate_another_pending_conversation() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let task = std::thread::spawn(move || {
        app.live_conversation_local(
            &account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-conversation-isolated-clear",
        )
    });
    let start = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(start.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.app
        .live_conversation_local(
            "other",
            C::Clear {
                view_id: "unknown".into(),
            },
            "owned-conversation-unrelated-clear",
        )
        .unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    assert!(task.join().unwrap().unwrap().is_some());
}

#[test]
fn conversation_read_excludes_media_transport_descriptors_from_all_public_pages() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    *f.reader.body.lock().unwrap() = "<msg><aeskey>owned-secret</aeskey></msg>".into();
    let first = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-conversation-private-open",
        )
        .unwrap()
        .unwrap();
    assert!(
        first
            .messages
            .rows
            .iter()
            .all(|r| r.metadata.is_none() && r.text.is_none() && r.content_status == "metadata")
    );
    let next = f
        .app
        .live_conversation_local(
            &f.account,
            C::Page {
                view_id: first.messages.view_id.clone(),
                cursor: first.messages.older.clone().unwrap(),
                direction: LiveDirection::Older,
            },
            "owned-conversation-private-page",
        )
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains("owned-secret")
    );
    assert!(!serde_json::to_string(&next).unwrap().contains("aeskey"));
}
#[test]
fn moments_page_cursor_context_and_cancellation_do_not_persist_plaintext() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let key_revision = f.repo.key_revision(&f.account).unwrap();
    let first = f
        .app
        .live_moments_local(&f.account, &view.view_id, None, "owned-moment-page-0001")
        .unwrap();
    assert_eq!(first.rows.len(), 30);
    assert!(!first.complete_timeline);
    assert!(!first.cross_page_snapshot);
    let next = first.next.as_deref().unwrap();
    let last = f
        .app
        .live_moments_local(
            &f.account,
            &view.view_id,
            Some(next),
            "owned-moment-page-0002",
        )
        .unwrap();
    assert_eq!(last.rows.len(), 1);
    assert!(last.next.is_none());
    let before = f.reader.reads.load(Ordering::SeqCst);
    for (account, id, cursor) in [
        (&*f.account, "wrong", None),
        ("other", &*view.view_id, None),
        (&*f.account, &*view.view_id, Some("bad-token")),
    ] {
        assert!(
            f.app
                .live_moments_local(account, id, cursor, "owned-moment-invalid")
                .is_err()
        );
    }
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), before);
    f.app.cancel_browse_local("owned-moment-canceled").unwrap();
    assert_eq!(
        f.app
            .live_moments_local(&f.account, &view.view_id, None, "owned-moment-canceled")
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.repo.key_revision(&f.account).unwrap(), key_revision);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let bytes = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(b"owned-moment-".len())
            .any(|w| w == b"owned-moment-")
    );
    let replacement = f.prepare().unwrap();
    assert!(
        f.app
            .live_moments_local(
                &f.account,
                &replacement.view_id,
                Some(next),
                "owned-moment-stale"
            )
            .is_err()
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .live_moments_local(
                &f.account,
                &replacement.view_id,
                None,
                "owned-moment-context"
            )
            .is_err()
    );
}

#[tokio::test]
async fn moments_http_checks_scope_range_and_revocation_after_read() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned moments",
            vec![f.account.clone()],
            vec!["moments:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/moments/live")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"view_id":view.view_id,"cursor":null,"request_id":"owned-moment-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 30);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other moment caller",
            vec![f.account.clone()],
            vec!["moments:read".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/moments/live/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-moment-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-moment-"));
}

#[test]
fn multi_conversation_merges_all_tied_ids_pages_and_windows_without_metadata_or_send() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &paths,
            "owned-multi-conversation-parent",
            true,
        )
        .unwrap();
    let mut page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id.clone(),
                username: "owned-A".into(),
            },
            "owned-multi-conversation-open",
        )
        .unwrap()
        .unwrap();
    assert!(
        f.app
            .recover_live_filehelper_multi_local(&f.account, &page.messages.view_id, true)
            .is_err()
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = vec![];
    let mut windows = 0;
    loop {
        assert_eq!(page.messages.source_count, 2);
        assert!(!page.sending_enabled);
        assert_eq!(page.source_scope, "selected_message_sources");
        for row in &page.messages.rows {
            assert!(seen.insert((row.source.clone(), row.message.local_id.clone())));
            assert!(row.message.metadata.is_none());
            if row.message.text.is_some() {
                assert_eq!(
                    row.message.text.as_deref(),
                    Some("owned-message-for-owned-A")
                );
            }
            ordered.push((
                row.message.timestamp,
                row.source.clone(),
                row.message.local_id.parse::<i64>().unwrap(),
            ));
        }
        let command = if let Some(cursor) = page.messages.older {
            C::Page {
                view_id: page.messages.view_id,
                cursor,
                direction: LiveDirection::Older,
            }
        } else if let Some(cursor) = page.messages.continuation {
            windows += 1;
            C::Continue {
                view_id: page.messages.view_id,
                cursor,
            }
        } else {
            break;
        };
        page = f
            .app
            .live_multi_conversation_local(&f.account, command, "owned-multi-conversation-next")
            .unwrap()
            .unwrap();
        assert!(windows < 10);
    }
    assert_eq!(seen.len(), 500);
    assert!(ordered.windows(2).all(|w| w[0] > w[1]));
    assert!(windows > 0);
    let old = page.messages.view_id.clone();
    f.app
        .live_multi_conversation_local(
            &f.account,
            C::Clear {
                view_id: old.clone(),
            },
            "owned-multi-conversation-clear",
        )
        .unwrap();
    assert!(
        f.app
            .live_multi_conversation_local(
                &f.account,
                C::Page {
                    view_id: old,
                    cursor: "bad".into(),
                    direction: LiveDirection::Older
                },
                "owned-multi-conversation-stale"
            )
            .is_err()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
}

#[test]
fn multi_session_cursor_and_conversation_context_bind_every_source_and_username() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &paths,
            "owned-multi-conversation-parent",
            true,
        )
        .unwrap();
    let sessions = f
        .app
        .live_sessions_local(
            &f.account,
            &parent.view_id,
            None,
            "owned-multi-conversation-sessions",
        )
        .unwrap();
    assert_eq!(sessions.rows.len(), 200);
    let next = f
        .app
        .live_sessions_local(
            &f.account,
            &parent.view_id,
            sessions.next.as_deref(),
            "owned-multi-conversation-sessions-next",
        )
        .unwrap();
    assert_eq!(next.rows.len(), 1);
    assert!(
        f.app
            .live_sessions_local(
                &f.account,
                &parent.view_id,
                parent.older.as_deref(),
                "owned-multi-conversation-mixed-cursor"
            )
            .is_err()
    );
    let open = |username: &str| C::Open {
        parent_view_id: parent.view_id.clone(),
        username: username.into(),
    };
    let first = f
        .app
        .live_multi_conversation_local(&f.account, open("owned-A"), "owned-multi-conversation-open")
        .unwrap()
        .unwrap();
    let second = f
        .app
        .live_multi_conversation_local(
            &f.account,
            open("owned-B"),
            "owned-multi-conversation-replace",
        )
        .unwrap()
        .unwrap();
    assert!(
        f.app
            .live_multi_conversation_local(
                &f.account,
                C::Page {
                    view_id: second.messages.view_id.clone(),
                    cursor: first.messages.older.unwrap(),
                    direction: LiveDirection::Older
                },
                "owned-multi-conversation-mixed-view"
            )
            .is_err()
    );
    assert!(
        f.app
            .live_multi_conversation_local(
                "other",
                open("owned-B"),
                "owned-multi-conversation-other-account"
            )
            .is_err()
    );
    let changed = std::path::Path::new(&paths[1]);
    let old = changed.with_file_name("old.db");
    std::fs::rename(changed, &old).unwrap();
    std::fs::copy(&old, changed).unwrap();
    assert!(
        f.app
            .live_sessions_local(
                &f.account,
                &parent.view_id,
                sessions.next.as_deref(),
                "owned-multi-conversation-source-change"
            )
            .is_err()
    );
    assert!(
        f.app
            .live_multi_conversation_local(
                &f.account,
                C::Page {
                    view_id: second.messages.view_id,
                    cursor: second.messages.older.unwrap(),
                    direction: LiveDirection::Older
                },
                "owned-multi-conversation-page-source-change"
            )
            .is_err()
    );
}

#[test]
fn clearing_multi_parent_cancels_pending_conversation_publication() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &paths,
            "owned-multi-conversation-parent",
            true,
        )
        .unwrap();
    f.reader.wait.store(true, Ordering::SeqCst);
    let before = f.reader.reads.load(Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let task = std::thread::spawn(move || {
        app.live_multi_conversation_local(
            &account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-multi-conversation-delayed",
        )
    });
    let start = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == before {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.app.clear_live_filehelper_local().unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    assert_eq!(task.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn multi_conversation_never_returns_transport_descriptors() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &two_sources(&f),
            "owned-multi-metadata-parent",
            true,
        )
        .unwrap();
    *f.reader.body.lock().unwrap() =
        "<msg><appmsg><aeskey>owned-transport-secret</aeskey></appmsg></msg>".into();
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-multi-metadata-open",
        )
        .unwrap()
        .unwrap();
    assert!(
        page.messages
            .rows
            .iter()
            .all(|r| r.message.metadata.is_none() && r.message.text.is_none())
    );
    assert!(
        !serde_json::to_string(&page)
            .unwrap()
            .contains("owned-transport-secret")
    );
}

#[test]
fn multi_conversation_accepts_initial_absence_but_rejects_lost_anchored_table() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    // Interleave timestamps so the first merged window anchors both sources.
    // With all timestamps tied the stable source tie-break can select only one.
    f.reader.spread_time.store(true, Ordering::SeqCst);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &two_sources(&f),
            "owned-multi-absence-parent",
            true,
        )
        .unwrap();
    let open = || C::Open {
        parent_view_id: parent.view_id.clone(),
        username: "owned-A".into(),
    };
    *f.reader.body.lock().unwrap() = "owned-conversation-missing-second".into();
    let page = f
        .app
        .live_multi_conversation_local(&f.account, open(), "owned-multi-absence-open")
        .unwrap()
        .unwrap();
    assert_eq!(
        page.messages
            .rows
            .iter()
            .map(|r| r.source_index)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1
    );
    *f.reader.body.lock().unwrap() = "owned-text".into();
    let mut page = f
        .app
        .live_multi_conversation_local(&f.account, open(), "owned-multi-both-open")
        .unwrap()
        .unwrap();
    while let Some(cursor) = page.messages.older {
        page = f
            .app
            .live_multi_conversation_local(
                &f.account,
                C::Page {
                    view_id: page.messages.view_id,
                    cursor,
                    direction: LiveDirection::Older,
                },
                "owned-multi-both-page",
            )
            .unwrap()
            .unwrap();
    }
    *f.reader.body.lock().unwrap() = "owned-conversation-missing-second".into();
    assert_eq!(
        f.app
            .live_multi_conversation_local(
                &f.account,
                C::Continue {
                    view_id: page.messages.view_id,
                    cursor: page.messages.continuation.unwrap()
                },
                "owned-multi-lost-table"
            )
            .err()
            .unwrap()
            .code,
        "LIVE_CONVERSATION_NOT_IN_SOURCE"
    );
}

#[tokio::test]
async fn group_http_checks_scope_range_token_isolated_cancel_and_output_revocation() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned groups",
            vec![f.account.clone()],
            vec!["contacts:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/contacts/live/group")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"view_id":view.view_id,"username":"owned@chatroom","cursor":null,"request_id":"owned-group-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 200);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other session caller",
            vec![f.account.clone()],
            vec!["contacts:read".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/contacts/live/group/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-group-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-group-"));
}

#[test]
fn group_members_page_is_bound_to_source_group_account_and_cursor_purpose() {
    for multi in [false, true] {
        let f = Fixture::new();
        let id = if multi {
            f.app
                .prepare_live_filehelper_multi_local(
                    &f.account,
                    &two_sources(&f),
                    "owned-group-multi-parent",
                    true,
                )
                .unwrap()
                .view_id
        } else {
            f.prepare().unwrap().view_id
        };
        let first = f
            .app
            .live_group_members_local(
                &f.account,
                &id,
                "owned@chatroom",
                None,
                "owned-group-first-page",
            )
            .unwrap();
        assert_eq!(first.rows.len(), 200);
        assert_eq!(
            first
                .rows
                .iter()
                .filter(|r| r.anchor.username.is_none())
                .count(),
            40
        );
        assert!(!first.complete_server_membership && !first.cross_page_snapshot);
        assert_eq!(first.cached_pins.as_ref().unwrap()[0].server_id,"18446744073709551615");
        let next = first.next.as_deref().unwrap();
        assert!(!String::from_utf8_lossy(&hex::decode(next).unwrap()).contains("owned@chatroom"));
        let second = f
            .app
            .live_group_members_local(
                &f.account,
                &id,
                "owned@chatroom",
                Some(next),
                "owned-group-second-page",
            )
            .unwrap();
        assert_eq!(second.rows.len(), 1);
        assert!(second.next.is_none());
        for (account, view, group, cursor) in [
            (&*f.account, &*id, "other@chatroom", Some(next)),
            ("other", &*id, "owned@chatroom", None),
            (&*f.account, "wrong-view", "owned@chatroom", None),
            (&*f.account, &*id, "filehelper", None),
        ] {
            assert!(
                f.app
                    .live_group_members_local(
                        account,
                        view,
                        group,
                        cursor,
                        "owned-group-invalid-case"
                    )
                    .is_err()
            );
        }
        assert!(
            f.app
                .live_sessions_local(&f.account, &id, Some(next), "owned-group-mixed-session")
                .is_err()
        );
        let sessions = f
            .app
            .live_sessions_local(&f.account, &id, None, "owned-group-session-page")
            .unwrap();
        assert!(
            f.app
                .live_group_members_local(
                    &f.account,
                    &id,
                    "owned@chatroom",
                    sessions.next.as_deref(),
                    "owned-group-mixed-purpose"
                )
                .is_err()
        );
        f.app.cancel_browse_local("owned-group-canceled").unwrap();
        assert_eq!(
            f.app
                .live_group_members_local(
                    &f.account,
                    &id,
                    "owned@chatroom",
                    None,
                    "owned-group-canceled"
                )
                .err()
                .unwrap()
                .code,
            "QUERY_CANCELED"
        );
        f.reader.invalid_group.store(true, Ordering::SeqCst);
        assert_eq!(
            f.app
                .live_group_members_local(
                    &f.account,
                    &id,
                    "owned@chatroom",
                    None,
                    "owned-group-bad-provider"
                )
                .err()
                .unwrap()
                .code,
            "LIVE_PROTOCOL_INVALID"
        );
        f.reader.invalid_group.store(false, Ordering::SeqCst);
        f.reader.change.store(true, Ordering::SeqCst);
        assert!(
            f.app
                .live_group_members_local(
                    &f.account,
                    &id,
                    "owned@chatroom",
                    None,
                    "owned-group-late-context"
                )
                .is_err()
        );
        assert!(f.repo.sends(&f.account).unwrap().is_empty());
        let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
        assert!(
            !disk
                .windows(b"owned-member-nickname".len())
                .any(|b| b == b"owned-member-nickname")
        );
    }
}

fn owned_emoticon() -> &'static [u8] {
    include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif")
}
fn owned_emoticon_xml() -> String {
    r#"<msg><emoji md5="df4813390984d6155aabb73ea22dade0" len="11181"/></msg>"#.into()
}
#[test]
fn live_emoticon_is_encrypted_and_survives_closed_store_reopen_without_sending_or_closing_gap() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    f.app.receive_once_local(&f.account).unwrap();
    let gap = f.repo.receive_cursor(&f.account).unwrap();
    let page = f.prepare().unwrap();
    let id = &page.rows[0].local_id;
    let asset = f
        .app
        .save_live_emoticon_local(&f.account, &page.view_id, id, "owned-emoticon-save-0001")
        .unwrap();
    assert_eq!(
        f.app
            .emoticon_contents_local(&f.account, &asset.id)
            .unwrap()
            .bytes
            .as_slice(),
        owned_emoticon()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert_eq!(f.repo.receive_cursor(&f.account).unwrap(), gap);
    let fpath = f.directory.join("state.sqlite3");
    let account = f.account.clone();
    let reader = f.reader.clone();
    let _reopen_directory=f._cleanup.clone();
    drop(f);
    reader.context.lock().unwrap().epoch = "disconnected".into();
    let repo = Arc::new(SqliteRepository::open(&fpath).unwrap());
    let app = Application::new(repo.clone(), reader);
    assert_eq!(
        app.emoticon_contents_local(&account, &asset.id)
            .unwrap()
            .bytes
            .as_slice(),
        owned_emoticon()
    );
    assert!(
        app.emoticon_contents_local("other-account", &asset.id)
            .is_err()
    );
    assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
    assert!(repo.sends(&account).unwrap().is_empty());
    let disk = std::fs::read(&fpath).unwrap();
    assert!(!disk.windows(64).any(|s| s == &owned_emoticon()[..64]));
}
#[test]
fn live_emoticon_rejects_wrong_rows_stale_views_missing_bytes_identity_change_and_cancel() {
    let f = Fixture::new();
    let p = f.prepare().unwrap();
    assert!(
        f.app
            .save_live_emoticon_local(
                &f.account,
                &p.view_id,
                &p.rows[0].local_id,
                "owned-emoticon-save-0001"
            )
            .is_err()
    );
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    let p = f.prepare().unwrap();
    let id = &p.rows[0].local_id;
    for (view, row) in [("stale", id.as_str()), (p.view_id.as_str(), "missing")] {
        assert!(
            f.app
                .save_live_emoticon_local(&f.account, view, row, "owned-emoticon-save-0002")
                .is_err()
        );
    }
    f.reader.voice_missing.store(true, Ordering::SeqCst);
    assert_eq!(
        f.app
            .save_live_emoticon_local(&f.account, &p.view_id, id, "owned-emoticon-save-0003")
            .unwrap_err()
            .code,
        "EMOTICON_CACHE_UNAVAILABLE"
    );
    f.reader.voice_missing.store(false, Ordering::SeqCst);
    f.app
        .cancel_browse_local("owned-emoticon-save-0004")
        .unwrap();
    assert_eq!(
        f.app
            .save_live_emoticon_local(&f.account, &p.view_id, id, "owned-emoticon-save-0004")
            .unwrap_err()
            .code,
        "QUERY_CANCELED"
    );
    f.reader.wait.store(true, Ordering::SeqCst);
    let before = f.reader.reads.load(Ordering::SeqCst);
    std::thread::scope(|s| {
        let call = s.spawn(|| {
            f.app
                .save_live_emoticon_local(&f.account, &p.view_id, id, "owned-emoticon-save-0005")
        });
        let started = std::time::Instant::now();
        while f.reader.reads.load(Ordering::SeqCst) == before {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        f.app
            .cancel_browse_local("owned-emoticon-save-0005")
            .unwrap();
        assert_eq!(call.join().unwrap().unwrap_err().code, "QUERY_CANCELED");
    });
    f.reader.wait.store(false, Ordering::SeqCst);
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .save_live_emoticon_local(&f.account, &p.view_id, id, "owned-emoticon-save-0006")
            .is_err()
    );
    assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[test]
fn live_emoticon_transaction_rejects_late_connection_and_key_revisions() {
    for changed_key in [false, true] {
        let f = Fixture::new();
        let a = f
            .app
            .upload_file_local(&f.account, "owned.gif", owned_emoticon().to_vec())
            .unwrap();
        let p = f.repo.file(&f.account, &a.id).unwrap();
        f.app.delete_file_local(&f.account, &a.id).unwrap();
        let revision = f
            .repo
            .current_connection()
            .unwrap()
            .revision
            .parse::<i64>()
            .unwrap();
        let key = f.repo.key_revision(&f.account).unwrap();
        if changed_key {
            f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
        }
        assert!(
            f.repo
                .save_live_file(&p, if changed_key { revision } else { revision + 1 }, key)
                .is_err()
        );
        assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
    }
}

#[tokio::test]
async fn emoticon_http_checks_scope_range_token_isolated_cancel_and_output_revocation() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    let view = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned groups",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/messages/live/emoticon")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"view_id":view.view_id,"local_id":view.rows[0].local_id,"request_id":"owned-emoticon-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["size"], 11181);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other session caller",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/messages/emoticon/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-emoticon-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-group-"));
}

#[test]
fn received_emoticon_relation_reopens_deletes_and_never_submits() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    f.app.receive_once_local(&f.account).unwrap();
    let rows = f.app.received_texts_local(&f.account, 0, 1).unwrap();
    let position = rows[0].position;
    assert!(
        f.app
            .received_emoticon_asset_local(&f.account, position)
            .unwrap()
            .is_none()
    );
    let gap = f.repo.receive_cursor(&f.account).unwrap();
    let asset = f
        .app
        .receive_emoticon_local(&f.account, position, "owned-emoticon-receive-0001")
        .unwrap();
    assert_eq!(
        f.app
            .received_emoticon_content_local(&f.account, position)
            .unwrap()
            .bytes
            .as_slice(),
        owned_emoticon()
    );
    assert!(
        f.app
            .received_file_asset_local(&f.account, position)
            .is_err()
    );
    assert_eq!(
        f.app
            .receive_emoticon_local(&f.account, position, "owned-emoticon-receive-0002")
            .unwrap()
            .id,
        asset.id
    );
    let directory = f.directory.clone();
    let account = f.account.clone();
    let reader = f.reader.clone();
    let _reopen_directory=f._cleanup.clone();
    drop(f);
    let repo = Arc::new(SqliteRepository::open(&directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo.clone(), reader);
    assert_eq!(
        app.received_emoticon_content_local(&account, position)
            .unwrap()
            .bytes
            .as_slice(),
        owned_emoticon()
    );
    assert!(
        app.received_emoticon_content_local("other", position)
            .is_err()
    );
    app.delete_received_emoticon_local(&account, position)
        .unwrap();
    assert!(
        app.received_emoticon_asset_local(&account, position)
            .unwrap()
            .is_none()
    );
    assert!(repo.sends(&account).unwrap().is_empty());
    assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
    assert!(
        !std::fs::read(directory.join("state.sqlite3"))
            .unwrap()
            .windows(64)
            .any(|b| b == &owned_emoticon()[..64])
    );
}
#[test]
fn received_emoticon_refuses_bad_hash_missing_cancel_and_identity_change() {
    for mode in ["hash", "missing", "cancel", "identity"] {
        let f = Fixture::new();
        *f.reader.body.lock().unwrap() = if mode == "hash" {
            owned_emoticon_xml().replace("df4813390984d6155aabb73ea22dade0", &"a".repeat(32))
        } else {
            owned_emoticon_xml()
        };
        f.app.receive_once_local(&f.account).unwrap();
        let pos = f.app.received_texts_local(&f.account, 0, 1).unwrap()[0].position;
        f.reader
            .voice_missing
            .store(mode == "missing", Ordering::SeqCst);
        f.reader.change.store(mode == "identity", Ordering::SeqCst);
        if mode == "cancel" {
            f.app
                .cancel_browse_local("owned-emoticon-fail-0001")
                .unwrap();
        }
        assert!(
            f.app
                .receive_emoticon_local(&f.account, pos, "owned-emoticon-fail-0001")
                .is_err()
        );
        assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
        assert!(f.repo.sends(&f.account).unwrap().is_empty());
    }
}

#[tokio::test]
async fn received_emoticon_http_binary_scope_delete_and_revoke() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    f.app.receive_once_local(&f.account).unwrap();
    let pos = f.app.received_texts_local(&f.account, 0, 1).unwrap()[0].position;
    let token = f
        .app
        .issue_token_local(
            "owned-emoticon",
            vec![f.account.clone()],
            vec![
                "messages:read".into(),
                "media:write".into(),
                "media:read".into(),
            ],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned-media-only",
            vec![f.account.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |method: &str, secret: &str, account: &str| {
        let uri = format!("/api/v1/messages/native/{pos}/emoticon?account_id={account}");
        Request::builder().method(method).uri(uri).header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
            .body(if method == "POST" { Body::from(serde_json::json!({"account_id":account,"request_id":"owned-emoticon-http-1001"}).to_string()) } else { Body::empty() }).unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", &token.secret, &f.account))
            .await
            .unwrap()
            .status(),
        404
    );
    for method in ["POST", "GET", "DELETE"] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(method, &wrong.secret, &f.account))
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request(method, &token.secret, "other"))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        router
            .clone()
            .oneshot(request("POST", &token.secret, &f.account))
            .await
            .unwrap()
            .status(),
        200
    );
    let response = router
        .clone()
        .oneshot(request("GET", &token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["content-type"], "image/gif");
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        owned_emoticon()
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", &token.secret, &f.account))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", &token.secret, &f.account))
            .await
            .unwrap()
            .status(),
        404
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    assert_eq!(
        router
            .oneshot(request("GET", &token.secret, &f.account))
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

fn owned_wxgf() -> &'static [u8] {
    include_bytes!("../../../tests/fixtures/owned-received-emoticon.wxgf")
}
fn owned_wxgf_xml() -> String {
    format!(
        "<msg><appmsg><type>8</type><appattach><totallen>2641358</totallen><emoticonmd5>{}</emoticonmd5></appattach></appmsg></msg>",
        "ab".repeat(16)
    )
}
#[test]
fn wxgf_receive_encrypted_original_and_offline_reopen_derived_preview() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_wxgf_xml();
    f.app.receive_once_local(&f.account).unwrap();
    let position = f.app.received_texts_local(&f.account, 0, 1).unwrap()[0].position;
    let gap = f.repo.receive_cursor(&f.account).unwrap();
    let asset = f
        .app
        .receive_emoticon_local(&f.account, position, "owned-wxgf-receive-0001")
        .unwrap();
    assert_eq!(asset.size, owned_wxgf().len());
    let preview = f
        .app
        .received_emoticon_content_local(&f.account, position)
        .unwrap();
    assert_eq!(preview.file_name, "received-emoticon.png");
    assert!(preview.bytes.starts_with(b"\x89PNG"));
    assert!(
        f.app
            .create_emoticon_send_local(
                &f.account,
                "filehelper",
                &asset.id,
                "wxgf-must-not-be-sent-as-gif"
            )
            .is_err()
    );
    assert!(
        f.app
            .received_file_asset_local(&f.account, position)
            .is_err()
    );
    let expected = preview.bytes.clone();
    let directory = f.directory.clone();
    let account = f.account.clone();
    let reader = f.reader.clone();
    let _reopen_directory=f._cleanup.clone();
    drop(f);
    let repo = Arc::new(SqliteRepository::open(&directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo.clone(), reader);
    assert_eq!(
        app.received_emoticon_content_local(&account, position)
            .unwrap()
            .bytes,
        expected
    );
    assert!(
        app.received_emoticon_content_local("other", position)
            .is_err()
    );
    assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
    assert!(repo.sends(&account).unwrap().is_empty());
    assert!(
        !std::fs::read(directory.join("state.sqlite3"))
            .unwrap()
            .windows(64)
            .any(|b| b == &owned_wxgf()[..64])
    );
    app.delete_received_emoticon_local(&account, position)
        .unwrap();
    assert!(
        app.received_emoticon_asset_local(&account, position)
            .unwrap()
            .is_none()
    );
}
#[test]
fn wxgf_decode_failure_never_commits_and_live_window_accepts_type8() {
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_wxgf_xml();
    let view = f.prepare().unwrap();
    f.reader.animation_error.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .save_live_emoticon_local(
                &f.account,
                &view.view_id,
                &view.rows[0].local_id,
                "owned-wxgf-live-0001"
            )
            .is_err()
    );
    assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
    f.reader.animation_error.store(false, Ordering::SeqCst);
    let asset = f
        .app
        .save_live_emoticon_local(
            &f.account,
            &view.view_id,
            &view.rows[0].local_id,
            "owned-wxgf-live-0002",
        )
        .unwrap();
    assert_eq!(asset.size, owned_wxgf().len());
    assert_eq!(
        f.app
            .emoticon_contents_local(&f.account, &asset.id)
            .unwrap()
            .file_name,
        "received-emoticon.png"
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[tokio::test]
async fn wxgf_preview_api_has_png_mime_and_requires_media_read() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_wxgf_xml();
    let view = f.prepare().unwrap();
    let asset = f
        .app
        .save_live_emoticon_local(
            &f.account,
            &view.view_id,
            &view.rows[0].local_id,
            "owned-wxgf-api-0001",
        )
        .unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned-wxgf",
            vec![f.account.clone()],
            vec!["media:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned-wrong",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let req = |key: &str| {
        Request::builder()
            .uri(format!(
                "/api/v1/media/emoticons/{}?account_id={}",
                asset.id, f.account
            ))
            .header("Authorization", format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(req(&wrong.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router.oneshot(req(&token.secret)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .starts_with(b"\x89PNG")
    );
}

fn selected_media(
    f: &Fixture,
    multi: bool,
) -> werc_application::conversation_media::ConversationMediaSelection {
    f.reader.spread_time.store(true, Ordering::SeqCst);
    use werc_application::{
        conversation_media::ConversationMediaSelection as S,
        live_conversation::ConversationCommand as C,
    };
    if multi {
        let p = f
            .app
            .prepare_live_filehelper_multi_local(
                &f.account,
                &two_sources(f),
                "owned-media-parent",
                true,
            )
            .unwrap();
        let mut p = f
            .app
            .live_multi_conversation_local(
                &f.account,
                C::Open {
                    parent_view_id: p.view_id,
                    username: "owned-selected-chat".into(),
                },
                "owned-media-open",
            )
            .unwrap()
            .unwrap();
        assert!(!p.media.is_empty() || !p.cards.is_empty());
        assert!(p.messages.rows.iter().all(|r| r.message.metadata.is_none()));
        while !p
            .messages
            .rows
            .iter()
            .any(|r| r.source_index == 2 && r.message.local_id == "250")
        {
            let cursor = p
                .messages
                .older
                .clone()
                .expect("selected source must occur in bounded window");
            p = f
                .app
                .live_multi_conversation_local(
                    &f.account,
                    C::Page {
                        view_id: p.messages.view_id.clone(),
                        cursor,
                        direction: LiveDirection::Older,
                    },
                    "owned-media-select-page",
                )
                .unwrap()
                .unwrap();
        }
        let r = p
            .messages
            .rows
            .iter()
            .find(|r| r.source_index == 2 && r.message.local_id == "250")
            .unwrap();
        S {
            view_id: p.messages.view_id,
            local_id: r.message.local_id.clone(),
            source: Some(r.source.clone()),
        }
    } else {
        let p = f.prepare().unwrap();
        let p = f
            .app
            .live_conversation_local(
                &f.account,
                C::Open {
                    parent_view_id: p.view_id,
                    username: "owned-selected-chat".into(),
                },
                "owned-media-open",
            )
            .unwrap()
            .unwrap();
        assert!(!p.media.is_empty() || !p.cards.is_empty());
        assert!(p.messages.rows.iter().all(|r| r.metadata.is_none()));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("aeskey"));
        assert!(!json.contains("cdnmidimgurl"));
        S {
            view_id: p.messages.view_id,
            local_id: p.messages.rows[0].local_id.clone(),
            source: None,
        }
    }
}
#[test]
fn selected_conversation_all_media_encrypt_reopen_and_delete_without_send_or_gap_change() {
    use werc_application::conversation_media::ConversationMediaKind as K;
    for (kind, body) in [
        (K::Voice, String::new()),
        (
            K::Image,
            format!(
                "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabb\" length=\"100\"/></msg>",
                "ab".repeat(16)
            ),
        ),
        (
            K::Video,
            format!(
                "<msg><videomsg aeskey=\"{}\" cdnvideourl=\"aabb\" length=\"100\"/></msg>",
                "ab".repeat(16)
            ),
        ),
        (
            K::File,
            format!(
                "<msg><appmsg><title>owned.txt</title><type>6</type><appattach><totallen>10</totallen><cdnattachurl>aabb</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
                "ab".repeat(16)
            ),
        ),
        (K::Emoticon, owned_emoticon_xml()),
        (K::Emoticon, owned_wxgf_xml()),
    ] {
        for multi in [false, true] {
            let f = Fixture::new();
            *f.reader.body.lock().unwrap() = body.clone();
            f.reader.voice.store(kind == K::Voice, Ordering::SeqCst);
            let selection = selected_media(&f, multi);
            let gap = f.repo.receive_cursor(&f.account).unwrap();
            let saved = f
                .app
                .save_conversation_media_local(&f.account, &selection, "owned-selected-media-save")
                .unwrap();
            assert_eq!(saved.reference.kind, kind);
            if kind == K::Video {
                let again = f
                    .app
                    .save_conversation_media_local(
                        &f.account,
                        &selection,
                        "owned-selected-media-again",
                    )
                    .unwrap();
                assert_eq!(again.reference.id, saved.reference.id);
                assert_eq!(again.sha256, saved.sha256);
            }

            if kind == K::Voice {
                let observed = f.reader.last_voice.lock().unwrap().clone().unwrap();
                assert_eq!(observed.0, "owned-selected-chat");
                assert_eq!(
                    PathBuf::from(observed.1).file_name().unwrap(),
                    if multi {
                        "message_1.db"
                    } else {
                        "message_0.db"
                    }
                );
            }
            let content = f
                .app
                .conversation_media_content_local(&f.account, &saved.reference)
                .unwrap();
            assert!(!content.file.bytes.is_empty());
            let expected = content.file.bytes.to_vec();
            let mime = content.mime;
            if kind == K::Voice {
                assert_eq!(expected, owned_voice_wav());
            }
            if kind == K::File {
                assert_eq!(expected, b"owned file");
            }
            let mut tampered = saved.reference.clone();
            tampered.kind = if kind == K::File { K::Image } else { K::File };
            assert!(
                f.app
                    .conversation_media_content_local(&f.account, &tampered)
                    .is_err()
            );
            assert!(
                f.app
                    .conversation_media_content_local("other-account", &saved.reference)
                    .is_err()
            );
            let db = f.directory.join("state.sqlite3");
            let account = f.account.clone();
            let reader = f.reader.clone();
            let _reopen_directory=f._cleanup.clone();
            drop(f);
            let repo = Arc::new(SqliteRepository::open(&db).unwrap());
            let app = Application::new(repo.clone(), reader);
            let reopened = app
                .conversation_media_content_local(&account, &saved.reference)
                .unwrap();
            assert_eq!(reopened.file.bytes.as_slice(), expected);
            assert_eq!(reopened.mime, mime);
            assert!(repo.sends(&account).unwrap().is_empty());
            assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
            app.delete_conversation_media_local(&account, &saved.reference)
                .unwrap();
            assert!(
                app.conversation_media_content_local(&account, &saved.reference)
                    .is_err()
            );
        }
    }
}
#[test]
fn selected_conversation_media_refuses_stale_wrong_source_and_canceled_work() {
    use werc_application::live_conversation::ConversationCommand as C;
    for multi in [false, true] {
        for mode in 0..7 {
            let f = Fixture::new();
            f.reader.voice.store(true, Ordering::SeqCst);
            let mut s = selected_media(&f, multi);
            match mode {
                0 => s.view_id = "wrong-view".into(),
                1 => s.local_id = "not-in-window".into(),
                2 => s.source = if multi { None } else { Some("bad-tag".into()) },
                3 => {
                    f.app
                        .cancel_browse_local("owned-selected-media-reject")
                        .unwrap();
                }
                4 => {
                    f.reader.context.lock().unwrap().epoch = "changed".into();
                }
                5 => {
                    f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
                }
                _ => {
                    let cmd = C::Clear {
                        view_id: s.view_id.clone(),
                    };
                    if multi {
                        f.app
                            .live_multi_conversation_local(&f.account, cmd, "owned-clear")
                            .unwrap();
                    } else {
                        f.app
                            .live_conversation_local(&f.account, cmd, "owned-clear")
                            .unwrap();
                    }
                }
            }
            assert!(
                f.app
                    .save_conversation_media_local(&f.account, &s, "owned-selected-media-reject")
                    .is_err()
            );
            assert!(f.app.audio_assets_local(&f.account).unwrap().is_empty());
        }
    }
}
#[test]
fn selected_conversation_clear_key_and_identity_during_media_read_prevent_commit() {
    use werc_application::live_conversation::ConversationCommand as C;
    for multi in [false, true] {
        for mode in 0..4 {
            let f = Fixture::new();
            f.reader.voice.store(true, Ordering::SeqCst);
            let s = selected_media(&f, multi);
            let view = s.view_id.clone();
            f.reader.wait.store(true, Ordering::SeqCst);
            let reads = f.reader.reads.load(Ordering::SeqCst);
            let app = f.app.clone();
            let account = f.account.clone();
            let task = std::thread::spawn(move || {
                app.save_conversation_media_local(&account, &s, "owned-selected-race")
            });
            let start = std::time::Instant::now();
            while f.reader.reads.load(Ordering::SeqCst) == reads {
                assert!(start.elapsed().as_secs() < 5);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            match mode {
                0 => {
                    let cmd = C::Clear { view_id: view };
                    if multi {
                        f.app
                            .live_multi_conversation_local(&f.account, cmd, "owned-clear")
                            .unwrap();
                    } else {
                        f.app
                            .live_conversation_local(&f.account, cmd, "owned-clear")
                            .unwrap();
                    }
                }
                1 => {
                    f.app.import_key_local(&f.account, "5a".repeat(32)).unwrap();
                }
                2 => {
                    f.reader.change.store(true, Ordering::SeqCst);
                }
                _ => {
                    f.app.cancel_browse_local("owned-selected-race").unwrap();
                }
            }
            f.reader.wait.store(false, Ordering::SeqCst);
            assert!(task.join().unwrap().is_err());
            assert!(f.app.audio_assets_local(&f.account).unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn conversation_media_http_checks_scope_range_token_isolated_cancel_and_output_revocation() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    let selection = selected_media(&f, false);
    let token = f
        .app
        .issue_token_local(
            "owned groups",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong scope",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/messages/live/conversation/media")
        .header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json")
        .body(Body::from(serde_json::json!({"account_id":account,"selection":{"view_id":selection.view_id,"local_id":selection.local_id,"source":selection.source},"request_id":"owned-emoticon-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["size"], 11181);
    f.reader.wait.store(true, Ordering::SeqCst);
    let count = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.clone().oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == count {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let other_token = f
        .app
        .issue_token_local(
            "other session caller",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let cancel = Request::builder()
        .method("POST")
        .uri("/api/v1/messages/live/conversation/media/cancel")
        .header("Authorization", format!("Bearer {}", other_token.secret))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"account_id":f.account,"request_id":"owned-emoticon-http-0001"})
                .to_string(),
        ))
        .unwrap();
    assert_eq!(router.oneshot(cancel).await.unwrap().status(), 200);
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        !future.is_finished(),
        "another token cannot cancel this read"
    );
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("owned-group-"));
}

#[tokio::test]
async fn conversation_media_http_content_is_binary_scoped_and_deletable() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_emoticon_xml();
    let s = selected_media(&f, true);
    let saved = f
        .app
        .save_conversation_media_local(&f.account, &s, "owned-media-http-save")
        .unwrap();
    let all = f
        .app
        .issue_token_local(
            "owned",
            vec![f.account.clone()],
            vec![
                "messages:read".into(),
                "media:read".into(),
                "media:write".into(),
            ],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned",
            vec![f.account.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let req = |secret: &str, account: &str, delete: bool| {
        Request::builder()
            .method(if delete { "DELETE" } else { "POST" })
            .uri(if delete {
                "/api/v1/messages/live/conversation/media"
            } else {
                "/api/v1/messages/live/conversation/media/content"
            })
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"account_id":account,"reference":saved.reference}).to_string(),
            ))
            .unwrap()
    };
    for delete in [false, true] {
        for (secret, account) in [
            (&*wrong.secret, &*f.account),
            (&*all.secret, "wrong-account"),
        ] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(req(secret, account, delete))
                    .await
                    .unwrap()
                    .status(),
                403
            );
        }
    }
    let response = router
        .clone()
        .oneshot(req(&all.secret, &f.account, false))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/gif");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        owned_emoticon()
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req(&all.secret, &f.account, true))
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(
        !router
            .oneshot(req(&all.secret, &f.account, false))
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

#[test]
fn same_local_id_in_merged_conversation_uses_exact_selected_source() {
    use werc_application::{
        conversation_media::ConversationMediaSelection, live_conversation::ConversationCommand as C,
    };
    let f = Fixture::new();
    f.reader.voice.store(true, Ordering::SeqCst);
    f.reader.spread_time.store(true, Ordering::SeqCst);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &two_sources(&f),
            "owned-source-parent",
            true,
        )
        .unwrap();
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "selected-other-chat".into(),
            },
            "owned-source-open",
        )
        .unwrap()
        .unwrap();
    let rows: Vec<_> = page
        .messages
        .rows
        .iter()
        .filter(|r| r.message.local_id == "250")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].source, rows[1].source);
    for row in rows {
        let s = ConversationMediaSelection {
            view_id: page.messages.view_id.clone(),
            local_id: "250".into(),
            source: Some(row.source.clone()),
        };
        f.app
            .save_conversation_media_local(&f.account, &s, "owned-source-save")
            .unwrap();
        let (chat, path) = f.reader.last_voice.lock().unwrap().clone().unwrap();
        assert_eq!(chat, "selected-other-chat");
        assert_eq!(
            PathBuf::from(path).file_name().unwrap(),
            format!("message_{}.db", row.source_index - 1).as_str()
        );
    }
}

#[test]
fn conversation_cards_single_and_merged_pages_bind_sources_and_hide_xml() {
    use werc_application::live_conversation::ConversationCommand as C;
    for body in [
        "<type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"5\"><weburlitem><title>owned forwarded link</title><link>https://example.test/</link></weburlitem><cdndatakey>transport-secret</cdndatakey></dataitem><dataitem datatype=\"6\"><locitem><lat>31.25</lat><lng>121.5</lng><poiname>owned place</poiname></locitem></dataitem></datalist></recordinfo>]]></recorditem>",
        "<type>3</type><title>owned music</title><sourcedisplayname>owned app</sourcedisplayname>",
        "<type>4</type><title>owned video</title>",
        "<type>33</type><title>owned mini program</title><weappinfo><appid>transport-secret</appid></weappinfo>",
        "<type>36</type><title>owned alternate mini program</title>",
        "<msg username=\"owned-contact\" nickname=\"Test\" ticket=\"transport-secret\"/>",
        "<msg><location x=\"31\" y=\"121\" label=\"Test\" mapurl=\"transport-secret\"/></msg>",
        "<type>5</type><title>owned link</title><des>summary</des><url>https://example.test/</url>",
        "<type>57</type><title>owned reply</title><refermsg><type>1</type><displayname>owned author</displayname><content>quoted text</content></refermsg>",
        "<type>19</type><title>owned forward</title><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><sourcename>owned author</sourcename><datadesc>forward text</datadesc></dataitem></datalist></recordinfo>]]></recorditem>",
        "<type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"17\"><recordxml><recordinfo><datalist><dataitem datatype=\"1\"><datadesc>nested owned text</datadesc></dataitem><dataitem datatype=\"8\"><cdndatakey>nested-transport-secret</cdndatakey></dataitem></datalist></recordinfo></recordxml></dataitem></datalist></recordinfo>]]></recorditem>",
    ] {
        let f = Fixture::new();
        f.reader.spread_time.store(true, Ordering::SeqCst);
        *f.reader.body.lock().unwrap() = if body.starts_with("<msg") {
            body.into()
        } else {
            format!(
                "<msg><appmsg>{body}<appattach><aeskey>transport-secret</aeskey></appattach></appmsg></msg>"
            )
        };
        f.reader
            .sender_envelope
            .store(body.contains("recordxml"), Ordering::SeqCst);
        let parent = f.prepare().unwrap();
        let first = f
            .app
            .live_conversation_local(
                &f.account,
                C::Open {
                    parent_view_id: parent.view_id,
                    username: "owned-A".into(),
                },
                "owned-cards-single-open",
            )
            .unwrap()
            .unwrap();
        assert_eq!(first.cards.len(), first.messages.rows.len());
        assert!(first.media.is_empty());
        if body.contains("weburlitem") {
            let data = serde_json::to_value(&first.cards[0].content).unwrap();
            assert_eq!(data["entries"][0]["detail"]["url"], "https://example.test/");
            assert_eq!(data["entries"][1]["detail"]["latitude"], "31.25");
            assert_eq!(data["entries"][1]["detail"]["place_name"], "owned place");
        }
        if body.contains("recordxml") {
            let data = serde_json::to_value(&first.cards[0].content).unwrap();
            assert_eq!(
                data["entries"][0]["nested"]["entries"][0]["text"],
                "nested owned text"
            );
            assert_eq!(data["entries"][0]["nested"]["entries"][1]["data_type"], 8);
            assert!(
                !serde_json::to_string(&data)
                    .unwrap()
                    .contains("nested-transport-secret")
            );
        }

        for (card, row) in first.cards.iter().zip(&first.messages.rows) {
            assert_eq!(card.local_id, row.local_id);
            assert!(card.source.is_none());
            assert!(row.metadata.is_none());
        }
        let second = f
            .app
            .live_conversation_local(
                &f.account,
                C::Page {
                    view_id: first.messages.view_id.clone(),
                    cursor: first.messages.older.clone().unwrap(),
                    direction: LiveDirection::Older,
                },
                "owned-cards-single-page",
            )
            .unwrap()
            .unwrap();
        assert_eq!(second.cards.len(), second.messages.rows.len());
        assert!(
            !serde_json::to_string(&second)
                .unwrap()
                .contains("transport-secret")
        );
        let another = f.database.with_file_name("message_1.db");
        std::fs::write(&another, vec![0; 4096]).unwrap();
        let paths = vec![
            f.database.to_str().unwrap().into(),
            another.to_str().unwrap().into(),
        ];
        let parent = f
            .app
            .prepare_live_filehelper_multi_local(
                &f.account,
                &paths,
                "owned-cards-multi-parent",
                true,
            )
            .unwrap();
        let merged = f
            .app
            .live_multi_conversation_local(
                &f.account,
                C::Open {
                    parent_view_id: parent.view_id,
                    username: "owned-A".into(),
                },
                "owned-cards-multi-open",
            )
            .unwrap()
            .unwrap();
        assert_eq!(merged.cards.len(), merged.messages.rows.len());
        let mut sources = std::collections::HashSet::new();
        for (card, row) in merged.cards.iter().zip(&merged.messages.rows) {
            assert_eq!(card.local_id, row.message.local_id);
            assert_eq!(card.source.as_deref(), Some(row.source.as_str()));
            assert!(row.message.metadata.is_none());
            sources.insert(row.source.clone());
        }
        assert_eq!(sources.len(), 2);
        assert!(
            !serde_json::to_string(&merged)
                .unwrap()
                .contains("transport-secret")
        );
        assert!(
            f.app
                .live_conversation_local(
                    "other",
                    C::Page {
                        view_id: first.messages.view_id,
                        cursor: first.messages.older.unwrap(),
                        direction: LiveDirection::Older
                    },
                    "owned-cards-other-account"
                )
                .is_err()
        );
    }
}

#[test]
fn invalid_card_retains_message_placeholder_without_raw_metadata() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap()="<msg><appmsg><type>57</type><title>transport-secret</title><refermsg><type>invalid</type></refermsg></appmsg></msg>".into();
    let parent = f.prepare().unwrap();
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-cards-invalid-open",
        )
        .unwrap()
        .unwrap();
    assert!(page.cards.is_empty());
    assert!(!page.messages.rows.is_empty());
    assert!(
        !serde_json::to_string(&page)
            .unwrap()
            .contains("transport-secret")
    );
}

#[test]
fn recipient_selection_requires_current_nonempty_authenticated_view_and_never_sends() {
    use werc_application::{
        live_conversation::ConversationCommand as C, recipients::RecipientSelection as S,
    };
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let select = |id: &str| S {
        view_id: id.into(),
        multi: false,
    };
    assert!(
        f.app
            .prepare_recipient_local(&f.account, select(&parent.view_id))
            .is_err()
    );
    let open = |name: &str| {
        f.app
            .live_conversation_local(
                &f.account,
                C::Open {
                    parent_view_id: parent.view_id.clone(),
                    username: name.into(),
                },
                "owned-recipient-open",
            )
            .unwrap()
            .unwrap()
    };
    let first = open("owned-selected@chatroom");
    assert!(
        f.app
            .prepare_recipient_local("other", select(&first.messages.view_id))
            .is_err()
    );
    let prepared = f
        .app
        .prepare_recipient_local(&f.account, select(&first.messages.view_id))
        .unwrap();
    assert_eq!(prepared.target_id, "owned-selected@chatroom");
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let empty = open("owned-empty-recipient");
    assert!(
        f.app
            .prepare_recipient_local(&f.account, select(&first.messages.view_id))
            .is_err()
    );
    assert!(
        f.app
            .prepare_recipient_local(&f.account, select(&empty.messages.view_id))
            .is_err()
    );
    let current = open("owned-other");
    f.reader.context.lock().unwrap().epoch = "identity-changed".into();
    assert!(
        f.app
            .prepare_recipient_local(&f.account, select(&current.messages.view_id))
            .is_err()
    );
    assert_eq!(
        *f.reader.recipients.lock().unwrap(),
        vec!["owned-selected@chatroom"]
    );
}
#[test]
fn multi_recipient_selection_checks_every_source_and_closing_view_rejects_new_preparation() {
    use werc_application::{
        live_conversation::ConversationCommand as C, recipients::RecipientSelection as S,
    };
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-recipient-multi", true)
        .unwrap();
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-multi-recipient".into(),
            },
            "owned-recipient-multi-open",
        )
        .unwrap()
        .unwrap();
    let select = || S {
        view_id: page.messages.view_id.clone(),
        multi: true,
    };
    assert_eq!(
        f.app
            .prepare_recipient_local(&f.account, select())
            .unwrap()
            .target_id,
        "owned-multi-recipient"
    );
    let second = std::path::Path::new(&paths[1]);
    std::fs::rename(second, second.with_extension("old")).unwrap();
    std::fs::copy(&f.database, second).unwrap();
    assert!(f.app.prepare_recipient_local(&f.account, select()).is_err());
    f.app
        .live_multi_conversation_local(
            &f.account,
            C::Clear {
                view_id: page.messages.view_id.clone(),
            },
            "owned-recipient-multi-clear",
        )
        .unwrap();
    assert!(f.app.prepare_recipient_local(&f.account, select()).is_err());
    assert_eq!(
        *f.reader.recipients.lock().unwrap(),
        vec!["owned-multi-recipient"]
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn mentions_bind_authenticated_group_page_and_encrypted_preview_without_send() {
    use werc_application::{
        live_conversation::ConversationCommand as C, mention_send::MentionSelection as M,
        recipients::RecipientSelection as R,
    };
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let group = "owned-mentions@chatroom";
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id.clone(),
                username: group.into(),
            },
            "owned-mentions-open",
        )
        .unwrap()
        .unwrap();
    let recipient = || R {
        view_id: page.messages.view_id.clone(),
        multi: false,
    };
    f.app
        .prepare_recipient_local(&f.account, recipient())
        .unwrap();
    let selection = |ids: Vec<String>| M {
        recipient: recipient(),
        parent_view_id: parent.view_id.clone(),
        cursor: None,
        members: ids,
    };
    let member = "member-1-owned-mentions".to_string();
    for ids in [
        vec!["unverified-member".into()],
        vec!["member-201-owned-mentions".into()],
        vec![member.clone(), member.clone()],
        vec!["@all".into()],
        vec![],
    ] {
        assert!(
            f.app
                .create_mention_send_local(
                    &f.account,
                    selection(ids),
                    "自动化测试",
                    "owned-mentions-invalid"
                )
                .is_err()
        );
    }
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    let draft = f
        .app
        .create_mention_send_local(
            &f.account,
            selection(vec![member.clone()]),
            "自动化测试，原生成员提醒",
            "owned-mentions-valid",
        )
        .unwrap();
    assert_eq!(draft.metadata.schema_version, 7);
    assert_eq!(draft.state, SendState::AwaitingApproval);
    let preview = f
        .app
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(preview.payload.mentions, vec![member.clone()]);
    assert_eq!(preview.payload.context.target_id, group);
    let again = f
        .app
        .create_mention_send_local(
            &f.account,
            selection(vec![member]),
            "自动化测试，原生成员提醒",
            "owned-mentions-valid",
        )
        .unwrap();
    assert_eq!(again.metadata.id, draft.metadata.id);
    assert!(
        f.app
            .create_mention_send_local(
                &f.account,
                selection(vec!["member-2-owned-mentions".into()]),
                "自动化测试，原生成员提醒",
                "owned-mentions-valid"
            )
            .is_err()
    );
    assert_eq!(f.repo.sends(&f.account).unwrap().len(), 1);
    f.app
        .live_conversation_local(
            &f.account,
            C::Clear {
                view_id: page.messages.view_id.clone(),
            },
            "owned-mentions-clear",
        )
        .unwrap();
    assert!(
        f.app
            .create_mention_send_local(
                &f.account,
                selection(vec!["member-2-owned-mentions".into()]),
                "自动化测试",
                "owned-mentions-after-clear"
            )
            .is_err()
    );
    assert_eq!(
        f.app
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap()
            .payload
            .mentions
            .len(),
        1
    );
}

#[test]
fn mentions_reject_identity_change_during_member_reread_and_cross_account() {
    use werc_application::{
        live_conversation::ConversationCommand as C, mention_send::MentionSelection as M,
        recipients::RecipientSelection as R,
    };
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let group = "owned-changing@chatroom";
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id.clone(),
                username: group.into(),
            },
            "owned-mention-changing-open",
        )
        .unwrap()
        .unwrap();
    let recipient = || R {
        view_id: page.messages.view_id.clone(),
        multi: false,
    };
    f.app
        .prepare_recipient_local(&f.account, recipient())
        .unwrap();
    let selection = || M {
        recipient: recipient(),
        parent_view_id: parent.view_id.clone(),
        cursor: None,
        members: vec!["member-1-owned-changing".into()],
    };
    assert!(
        f.app
            .create_mention_send_local(
                "other-account",
                selection(),
                "自动化测试",
                "owned-mention-other-account"
            )
            .is_err()
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .create_mention_send_local(
                &f.account,
                selection(),
                "自动化测试",
                "owned-mention-changing"
            )
            .is_err()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn quote_source_is_authenticated_encrypted_and_submitted_once_without_plain_fallback() {
    use werc_application::{
        conversation_media::ConversationMediaSelection as S,
        live_conversation::ConversationCommand as C, recipients::RecipientSelection,
    };
    let f = Fixture::new();
    f.reader.compressed_text.store(true, Ordering::SeqCst);
    let parent = f.prepare().unwrap();
    let target = "owned-quote";
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id.clone(),
                username: target.into(),
            },
            "owned-quote-view",
        )
        .unwrap()
        .unwrap();
    f.app
        .prepare_recipient_local(
            &f.account,
            RecipientSelection {
                view_id: page.messages.view_id.clone(),
                multi: false,
            },
        )
        .unwrap();
    let selection = S {
        view_id: page.messages.view_id.clone(),
        local_id: page.messages.rows[0].local_id.clone(),
        source: None,
    };
    let reply = "中".repeat(3333) + "x";
    let draft = f
        .app
        .create_quote_send_local(&f.account, &selection, &reply, "owned-quote-draft")
        .unwrap();
    assert_eq!(draft.metadata.schema_version, 8);
    assert_eq!(draft.state, SendState::AwaitingApproval);
    let preview = f
        .app
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    let source = preview.payload.quote.as_ref().unwrap();
    assert_eq!(
        source.server_id,
        page.messages.rows[0].server_id.as_deref().unwrap()
    );
    assert!(source.server_id.parse::<u64>().unwrap() > 9007199254740991);
    assert_eq!(source.text, "owned-message-for-owned-quote");
    assert_eq!(preview.payload.text, reply);
    assert_eq!(
        f.app
            .create_quote_send_local(&f.account, &selection, &reply, "owned-quote-draft")
            .unwrap()
            .metadata
            .id,
        draft.metadata.id
    );
    assert!(
        f.app
            .create_quote_send_local(
                &f.account,
                &selection,
                "different reply",
                "owned-quote-draft"
            )
            .is_err()
    );
    assert!(
        f.app
            .create_quote_send_local(
                &f.account,
                &selection,
                &("x".repeat(10001)),
                "owned-quote-too-long"
            )
            .is_err()
    );
    let changed = S {
        local_id: page.messages.rows[1].local_id.clone(),
        ..S {
            view_id: selection.view_id.clone(),
            local_id: String::new(),
            source: None,
        }
    };
    assert!(
        f.app
            .create_quote_send_local(&f.account, &changed, &reply, "owned-quote-draft")
            .is_err()
    );
    assert_eq!(f.reader.quote_calls.load(Ordering::SeqCst), 0);
    f.app
        .live_conversation_local(
            &f.account,
            C::Clear {
                view_id: page.messages.view_id,
            },
            "owned-quote-clear",
        )
        .unwrap();
    assert!(
        f.app
            .create_quote_send_local(&f.account, &selection, &reply, "owned-quote-after-clear")
            .is_err()
    );
    assert_eq!(
        f.app
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap()
            .payload
            .quote
            .as_ref()
            .unwrap()
            .text,
        source.text
    );
    f.app
        .approve_text_send_local(
            &f.account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )
        .unwrap();
    let sent = f
        .app
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(sent.state, SendState::Unknown);
    assert!(sent.attempt_id.is_some());
    assert!(
        f.app
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.reader.quote_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn quote_rejects_cross_account_source_and_stale_identity() {
    use werc_application::{
        conversation_media::ConversationMediaSelection as S,
        live_conversation::ConversationCommand as C, recipients::RecipientSelection,
    };
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-quote".into(),
            },
            "owned-quote-invalid-view",
        )
        .unwrap()
        .unwrap();
    f.app
        .prepare_recipient_local(
            &f.account,
            RecipientSelection {
                view_id: page.messages.view_id.clone(),
                multi: false,
            },
        )
        .unwrap();
    let mut selection = S {
        view_id: page.messages.view_id.clone(),
        local_id: page.messages.rows[0].local_id.clone(),
        source: Some("forged-source".into()),
    };
    assert!(
        f.app
            .create_quote_send_local(&f.account, &selection, "reply", "owned-quote-forged-source")
            .is_err()
    );
    selection.source = None;
    assert!(
        f.app
            .create_quote_send_local(
                "another-account",
                &selection,
                "reply",
                "owned-quote-other-account"
            )
            .is_err()
    );
    f.reader.context.lock().unwrap().epoch = "changed-before-quote".into();
    assert!(
        f.app
            .create_quote_send_local(
                &f.account,
                &selection,
                "reply",
                "owned-quote-changed-identity"
            )
            .is_err()
    );
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
    assert_eq!(f.reader.quote_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn quote_merged_selection_retains_source_tag_and_rejects_missing_tag() {
    use werc_application::{
        conversation_media::ConversationMediaSelection as S,
        live_conversation::ConversationCommand as C, recipients::RecipientSelection,
    };
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-quote-multi-parent", true)
        .unwrap();
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-multi-quote".into(),
            },
            "owned-quote-multi-open",
        )
        .unwrap()
        .unwrap();
    f.app
        .prepare_recipient_local(
            &f.account,
            RecipientSelection {
                view_id: page.messages.view_id.clone(),
                multi: true,
            },
        )
        .unwrap();
    let row = &page.messages.rows[0];
    let mut selection = S {
        view_id: page.messages.view_id.clone(),
        local_id: row.message.local_id.clone(),
        source: None,
    };
    assert!(
        f.app
            .create_quote_send_local(&f.account, &selection, "reply", "owned-quote-no-tag")
            .is_err()
    );
    selection.source = Some("wrong-tag".into());
    assert!(
        f.app
            .create_quote_send_local(&f.account, &selection, "reply", "owned-quote-wrong-tag")
            .is_err()
    );
    selection.source = Some(row.source.clone());
    let draft = f
        .app
        .create_quote_send_local(&f.account, &selection, "reply", "owned-quote-multi-draft")
        .unwrap();
    let preview = f
        .app
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(
        preview.payload.quote.as_ref().unwrap().server_id,
        row.message.server_id.as_deref().unwrap()
    );
    assert_eq!(preview.payload.context.target_id, "owned-multi-quote");
    f.app
        .cancel_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(f.reader.quote_calls.load(Ordering::SeqCst), 0);
}

fn owned_forward_image_xml() -> String {
    format!(
        "<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><datadesc>owned text</datadesc></dataitem><dataitem datatype=\"2\"><cdndataurl>aabb</cdndataurl><cdndatakey>{}</cdndatakey><datasize>130</datasize><fullmd5>22f9f424c04f42866b833a812ebe4605</fullmd5></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>",
        "ab".repeat(16)
    )
}
#[test]
fn forwarded_image_selected_source_encrypts_reopens_and_deletes() {
    for multi in [false, true] {
        let f = Fixture::new();
        *f.reader.body.lock().unwrap() = owned_forward_image_xml();
        let selection = selected_media(&f, multi);
        let gap = f.repo.receive_cursor(&f.account).unwrap();
        let asset = f
            .app
            .save_conversation_media_entry_local(
                &f.account,
                &selection,
                Some(1),
                "owned-forward-image-save",
            )
            .unwrap();
        assert_eq!(
            asset.reference.kind,
            werc_application::conversation_media::ConversationMediaKind::Image
        );
        let bytes = f
            .app
            .conversation_media_content_local(&f.account, &asset.reference)
            .unwrap()
            .file
            .bytes
            .to_vec();
        werc_domain::image::canonical_received_png(&bytes).unwrap();
        let db = f.directory.join("state.sqlite3");
        let account = f.account.clone();
        let reader = f.reader.clone();
        let _reopen_directory=f._cleanup.clone();
        drop(f);
        let repo = Arc::new(SqliteRepository::open(&db).unwrap());
        let app = Application::new(repo.clone(), reader);
        assert_eq!(
            app.conversation_media_content_local(&account, &asset.reference)
                .unwrap()
                .file
                .bytes
                .as_slice(),
            bytes
        );
        assert!(repo.sends(&account).unwrap().is_empty());
        assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
        app.delete_conversation_media_local(&account, &asset.reference)
            .unwrap();
        assert!(
            app.conversation_media_content_local(&account, &asset.reference)
                .is_err()
        );
    }
}
#[test]
fn forwarded_image_rejects_other_entries_credentials_integrity_stale_and_canceled() {
    for multi in [false, true] {
        for mode in 0..8 {
            let f = Fixture::new();
            let xml = owned_forward_image_xml();
            *f.reader.body.lock().unwrap() = if mode == 4 {
                xml.replace(
                    "22f9f424c04f42866b833a812ebe4605",
                    "00000000000000000000000000000000",
                )
            } else {
                xml
            };
            let mut selection = selected_media(&f, multi);
            if mode == 5 {
                selection.source = Some("wrong-source".into());
            }
            if mode == 6 {
                selection.view_id = "wrong-view".into();
            }
            if mode == 7 {
                f.app.cancel_browse_local("owned-forward-reject").unwrap();
            }
            let index = match mode {
                0 => None,
                1 => Some(0),
                2 => Some(2),
                3 => Some(100),
                _ => Some(1),
            };
            assert!(
                f.app
                    .save_conversation_media_entry_local(
                        &f.account,
                        &selection,
                        index,
                        "owned-forward-reject"
                    )
                    .is_err()
            );
            assert!(f.repo.sends(&f.account).unwrap().is_empty());
            assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn forward_image_http_requires_scope_and_never_accepts_credentials() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_forward_image_xml();
    let selection = selected_media(&f, true);
    let all = f
        .app
        .issue_token_local(
            "owned-forward",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned-no-media",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let body = serde_json::json!({"account_id":f.account,"selection":{"view_id":selection.view_id,"local_id":selection.local_id,"source":selection.source},"forward_index":1,"request_id":"owned-forward-http-0001"});
    let request = |secret: &str, value: &serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/messages/live/conversation/media")
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(&wrong.secret, &body))
            .await
            .unwrap()
            .status(),
        403
    );
    let mut forged = body.clone();
    forged["aes_key"] = "untrusted".into();
    assert_eq!(
        router
            .clone()
            .oneshot(request(&all.secret, &forged))
            .await
            .unwrap()
            .status(),
        422
    );
    let mut wrong_index = body.clone();
    wrong_index["forward_index"] = 0.into();
    assert_eq!(
        router
            .clone()
            .oneshot(request(&all.secret, &wrong_index))
            .await
            .unwrap()
            .status(),
        415
    );
    let response = router.oneshot(request(&all.secret, &body)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(data["reference"]["kind"], "image");
    assert!(!String::from_utf8_lossy(&bytes).contains("cdndatakey"));
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn sender_enveloped_cards_and_media_share_the_authenticated_body() {
    use werc_application::conversation_media::ConversationMediaKind as K;
    for (body, index, expected_kind) in [
        (owned_forward_image_xml(), Some(1), K::Image),
        (
            format!(
                "<msg><appmsg><title>owned.txt</title><type>6</type><appattach><totallen>10</totallen><cdnattachurl>aabb</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
                "ab".repeat(16)
            ),
            None,
            K::File,
        ),
        (owned_emoticon_xml(), None, K::Emoticon),
    ] {
        for multi in [false, true] {
            let f = Fixture::new();
            *f.reader.body.lock().unwrap() = body.clone();
            f.reader.sender_envelope.store(true, Ordering::SeqCst);
            let selection = selected_media(&f, multi);
            let asset = f
                .app
                .save_conversation_media_entry_local(
                    &f.account,
                    &selection,
                    index,
                    "owned-envelope-save",
                )
                .unwrap();
            assert_eq!(asset.reference.kind, expected_kind);
            assert!(
                !f.app
                    .conversation_media_content_local(&f.account, &asset.reference)
                    .unwrap()
                    .file
                    .bytes
                    .is_empty()
            );
            assert!(f.repo.sends(&f.account).unwrap().is_empty());
            f.app
                .delete_conversation_media_local(&f.account, &asset.reference)
                .unwrap();
        }
    }
}

fn owned_forward_file_xml() -> String {
    let file = "<dataitem datatype=\"8\"><datatitle>自动化测试.txt</datatitle><datasize>3</datasize><fullmd5>900150983cd24fb0d6963f7d28e17f72</fullmd5></dataitem>";
    format!(
        "<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist>{file}<dataitem datatype=\"17\"><recordxml><recordinfo><datalist>{file}</datalist></recordinfo></recordxml></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>"
    )
}
#[test]
fn forwarded_files_select_exact_nested_source_encrypt_reopen_and_delete() {
    for multi in [false, true] {
        for path in [vec![0], vec![1, 0]] {
            let f = Fixture::new();
            *f.reader.body.lock().unwrap() = owned_forward_file_xml();
            let selection = selected_media(&f, multi);
            let gap = f.repo.receive_cursor(&f.account).unwrap();
            let asset = f
                .app
                .save_conversation_media_path_local(
                    &f.account,
                    &selection,
                    None,
                    Some(&path),
                    "owned-forward-file-save",
                )
                .unwrap();
            assert_eq!(
                asset.reference.kind,
                werc_application::conversation_media::ConversationMediaKind::File
            );
            let original = f
                .app
                .conversation_media_content_local(&f.account, &asset.reference)
                .unwrap();
            assert_eq!(original.file.bytes.as_slice(), b"abc");
            assert_eq!(original.file.file_name, "自动化测试.txt");
            let db = f.directory.join("state.sqlite3");
            let account = f.account.clone();
            let reader = f.reader.clone();
            let _reopen_directory=f._cleanup.clone();
            drop(f);
            let repo = Arc::new(SqliteRepository::open(&db).unwrap());
            let app = Application::new(repo.clone(), reader);
            let reopened = app
                .conversation_media_content_local(&account, &asset.reference)
                .unwrap();
            assert_eq!(
                reopened.file.bytes.as_slice(),
                original.file.bytes.as_slice()
            );
            assert_eq!(reopened.file.file_name, original.file.file_name);
            assert!(repo.sends(&account).unwrap().is_empty());
            assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
            app.delete_conversation_media_local(&account, &asset.reference)
                .unwrap();
            assert!(
                app.conversation_media_content_local(&account, &asset.reference)
                    .is_err()
            );
        }
    }
}
#[test]
fn forwarded_files_reject_wrong_path_source_integrity_cancellation_and_context_change() {
    for multi in [false, true] {
        for mode in 0..11 {
            let f = Fixture::new();
            *f.reader.body.lock().unwrap() = if mode == 5 {
                owned_forward_file_xml().replace(
                    "900150983cd24fb0d6963f7d28e17f72",
                    "00000000000000000000000000000000",
                )
            } else {
                owned_forward_file_xml()
            };
            let mut selection = selected_media(&f, multi);
            if mode == 6 {
                selection.source = Some("wrong-source".into());
            }
            if mode == 7 {
                selection.view_id = "wrong-view".into();
            }
            if mode == 8 {
                f.app
                    .cancel_browse_local("owned-forward-file-reject")
                    .unwrap();
            }
            if mode == 9 {
                f.reader.voice_missing.store(true, Ordering::SeqCst);
            }
            if mode == 10 {
                f.reader.change.store(true, Ordering::SeqCst);
            }
            let path = match mode {
                0 => vec![],
                1 => vec![1],
                2 => vec![0, 0],
                3 => vec![100],
                _ => vec![1, 0],
            };
            assert!(
                f.app
                    .save_conversation_media_path_local(
                        &f.account,
                        &selection,
                        if mode == 4 { Some(0) } else { None },
                        Some(&path),
                        "owned-forward-file-reject"
                    )
                    .is_err()
            );
            assert!(f.repo.sends(&f.account).unwrap().is_empty());
            assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
        }
    }
}
#[tokio::test]
async fn forward_file_http_paths_are_exclusive_and_credentials_are_not_client_inputs() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = owned_forward_file_xml();
    let selection = selected_media(&f, true);
    let all = f
        .app
        .issue_token_local(
            "owned-forward-file",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned-forward-no-media",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let body = serde_json::json!({"account_id":f.account,"selection":{"view_id":selection.view_id,"local_id":selection.local_id,"source":selection.source},"forward_path":[1,0],"request_id":"owned-forward-file-http"});
    let req = |token: &str, v: &serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/messages/live/conversation/media")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(req(&wrong.secret, &body))
            .await
            .unwrap()
            .status(),
        403
    );
    for field in ["file_name", "database", "md5", "aes_key"] {
        let mut v = body.clone();
        v[field] = "untrusted".into();
        assert_eq!(
            router
                .clone()
                .oneshot(req(&all.secret, &v))
                .await
                .unwrap()
                .status(),
            422
        );
    }
    let mut both = body.clone();
    both["forward_index"] = 0.into();
    assert_eq!(
        router
            .clone()
            .oneshot(req(&all.secret, &both))
            .await
            .unwrap()
            .status(),
        400
    );
    f.reader.voice_missing.store(true, Ordering::SeqCst);
    assert_eq!(
        router
            .clone()
            .oneshot(req(&all.secret, &body))
            .await
            .unwrap()
            .status(),
        404
    );
    f.reader.voice_missing.store(false, Ordering::SeqCst);
    let response = router.oneshot(req(&all.secret, &body)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["reference"]["kind"], "file");
    assert!(!String::from_utf8_lossy(&bytes).contains("自动化测试.txt"));
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn source_query_finds_old_message_and_restores_both_sides_of_context() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let opened = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-query-open",
        )
        .unwrap()
        .unwrap();
    let mut query = LiveMessageQuery::default();
    query.server_id = Some((9007199254740992_i64 + 10).to_string());
    let found = f
        .app
        .live_conversation_local(
            &f.account,
            C::Find {
                view_id: opened.messages.view_id,
                query,
            },
            "owned-query-find",
        )
        .unwrap()
        .unwrap();
    assert_eq!(found.messages.rows.len(), 1);
    assert_eq!(found.messages.rows[0].local_id, "10");
    let context = f
        .app
        .live_conversation_local(
            &f.account,
            C::Context {
                view_id: found.messages.view_id,
                local_id: "10".into(),
                source: None,
            },
            "owned-query-context",
        )
        .unwrap()
        .unwrap();
    assert_eq!(context.messages.rows.len(), 40);
    assert_eq!(context.messages.rows[0].local_id, "40");
    assert_eq!(context.messages.rows[30].local_id, "10");
    assert_eq!(context.messages.rows.last().unwrap().local_id, "1");
    assert!(context.query.is_none());
    assert_eq!(context.focus.unwrap().local_id, "10");
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
#[test]
fn merged_context_keeps_same_second_source_order_and_exact_focus() {
    use werc_application::live_conversation::ConversationCommand as C;
    let f = Fixture::new();
    let paths = two_sources(&f);
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(&f.account, &paths, "owned-context-parent", true)
        .unwrap();
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: "owned-A".into(),
            },
            "owned-context-open",
        )
        .unwrap()
        .unwrap();
    let chosen = &page.messages.rows[45];
    let id = chosen.message.local_id.clone();
    let tag = chosen.source.clone();
    let context = f
        .app
        .live_multi_conversation_local(
            &f.account,
            C::Context {
                view_id: page.messages.view_id.clone(),
                local_id: id.clone(),
                source: Some(tag.clone()),
            },
            "owned-context-read",
        )
        .unwrap()
        .unwrap();
    assert_eq!(context.messages.rows[30].message.local_id, id);
    assert_eq!(context.messages.rows[30].source, tag);
    let keys = context
        .messages
        .rows
        .iter()
        .map(|r| {
            (
                r.message.timestamp,
                r.source.as_str(),
                r.message.local_id.parse::<i64>().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|w| w[0] > w[1]));
    let mut page = context;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        for row in &page.messages.rows {
            assert!(seen.insert((row.source.clone(), row.message.local_id.clone())));
        }
        if let Some(cursor) = page.messages.older.clone() {
            page = f
                .app
                .live_multi_conversation_local(
                    &f.account,
                    C::Page {
                        view_id: page.messages.view_id.clone(),
                        cursor,
                        direction: LiveDirection::Older,
                    },
                    "owned-context-page",
                )
                .unwrap()
                .unwrap();
        } else if let Some(cursor) = page.messages.continuation.clone() {
            page = f
                .app
                .live_multi_conversation_local(
                    &f.account,
                    C::Continue {
                        view_id: page.messages.view_id.clone(),
                        cursor,
                    },
                    "owned-context-continue",
                )
                .unwrap()
                .unwrap();
        } else {
            break;
        }
    }
    assert_eq!(seen.len(), 485); // 500 source rows, minus the 15 beyond nearest newer context.
}

#[path = "live_read/moments_tools.rs"]
mod moments_tools;
