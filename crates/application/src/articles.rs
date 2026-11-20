//! Encrypted article previews, explicit save/discard and portable exports.
use crate::Application;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, articles::*};
const KIND: &str = "web_article";
#[derive(Clone, Serialize, Deserialize)]
pub struct ArticleRecord {
    pub saved: Option<Article>,
    pub pending: Option<Article>,
    #[serde(default)]
    pub owner_run: Option<String>,
}
#[derive(Serialize)]
pub struct ArticleItem {
    pub id: String,
    pub revision: i64,
    pub record: ArticleRecord,
}
#[derive(Serialize)]
pub struct ArticlePage {
    pub data: Vec<ArticleItem>,
    pub next_cursor: Option<i64>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArticleAction {
    CacheImage { id:String, index:usize },
    Images { id:String },
    Image { id:String },
    History { publisher:String, cursor:Option<String> },
    List {
        before: Option<i64>,
    },
    Get {
        id: String,
    },
    Fetch {
        url: String,
    },
    Confirm {
        id: String,
        revision: i64,
        save: bool,
    },
    Delete {
        id: String,
        revision: i64,
    },
    CaptureLinks {
        capture_id: String,
        page: u64,
    },
}
impl ArticleAction {
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Self::CacheImage { .. } | Self::Fetch { .. } | Self::Confirm { .. } | Self::Delete { .. }
        )
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArticleExportFormat {
    Zip,
    Json,
    Md,
    Html,
    Xlsx,
}
impl ArticleExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Json => "json",
            Self::Md => "md",
            Self::Html => "html",
            Self::Xlsx => "xlsx",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Self::Zip => "application/zip",
            Self::Json => "application/json",
            Self::Md => "text/markdown; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }
}
pub struct ArticleExport {
    pub bytes: Vec<u8>,
    pub rows: usize,
    pub external_images: usize,
}
fn error() -> AppError {
    AppError::new("DOCUMENT_CORRUPTED", "文章副本完整性校验失败")
}
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
impl Application {
    fn article_editable(&self, account: &str, record: &ArticleRecord) -> AppResult<()> {
        if let Some(id) = &record.owner_run {
            let run = self.article_run_local(account, id)?;
            if matches!(
                run.run.state.as_str(),
                "queued" | "reading" | "pending" | "exporting"
            ) {
                return Err(AppError::new("BUSY", "请在文章任务中确认或放弃这批预览"));
            }
        }
        Ok(())
    }

    pub fn article_local(&self, account: &str, id: &str) -> AppResult<ArticleItem> {
        let (h, record): (_, ArticleRecord) = self.open_document(account, id, KIND)?;
        if record.saved.as_ref().is_some_and(|v| !v.valid())
            || record.pending.as_ref().is_some_and(|v| !v.valid())
        {
            return Err(error());
        }
        Ok(ArticleItem {
            id: h.id,
            revision: h.revision,
            record,
        })
    }
    pub fn articles_local(&self, account: &str, before: Option<i64>) -> AppResult<ArticlePage> {
        self.ensure_account(account)?;
        let page = self.repository.library_list(account, KIND, before, 20)?;
        Ok(ArticlePage {
            data: page
                .data
                .iter()
                .map(|h| self.article_local(account, &h.id))
                .collect::<AppResult<_>>()?,
            next_cursor: page.next_cursor,
        })
    }
    pub fn fetch_article_local(
        &self,
        account: &str,
        value: &str,
        request: &str,
    ) -> AppResult<ArticleItem> {
        self.ensure_account(account)?;
        let url = canonical_url(value)?;
        let lease = self.browse_lease(request)?;
        // The public reader intentionally needs no exported WeChat credentials.
        let article = self.provider.fetch_article(account, &url, &lease.cancel)?;
        lease.check()?;
        if !article.valid() {
            return Err(error());
        }
        let id = format!(
            "article-{}",
            hex::encode(Sha256::digest(article.url.as_bytes()))
        );
        let mut item = match self.article_local(account, &id) {
            Ok(item) => item,
            Err(e) if e.code == "NOT_FOUND" => ArticleItem {
                id: id.clone(),
                revision: 0,
                record: ArticleRecord {
                    saved: None,
                    pending: None,
                    owner_run: None,
                },
            },
            Err(e) => return Err(e),
        };
        item.record.pending = Some(article);
        item.revision += 1;
        let doc = self.seal_document(account, &id, KIND, item.revision, &item.record)?;
        lease.check()?;
        self.repository.library_save(&[doc], None)?;
        Ok(item)
    }
    pub fn confirm_article_local(
        &self,
        account: &str,
        id: &str,
        revision: i64,
        save: bool,
    ) -> AppResult<ArticleItem> {
        let mut item = self.article_local(account, id)?;
        self.article_editable(account, &item.record)?;
        if item.revision != revision {
            return Err(AppError::new("DOCUMENT_CONFLICT", "文章已更新，请刷新"));
        }
        let pending = item
            .record
            .pending
            .take()
            .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "该文章没有待确认内容"))?;
        if save {
            item.record.saved = Some(pending);
        }
        item.revision += 1;
        self.repository.library_save(
            &[self.seal_document(account, id, KIND, item.revision, &item.record)?],
            None,
        )?;
        Ok(item)
    }
    pub fn article_action_local(
        &self,
        account: &str,
        action: ArticleAction,
        request: &str,
    ) -> AppResult<serde_json::Value> {
        use serde_json::to_value;
        match action {
            ArticleAction::CacheImage{id,index}=>to_value(self.cache_article_image_local(account,&id,index,request)?),
            ArticleAction::Images{id}=>{let a=self.article_local(account,&id)?.record.saved.ok_or_else(||AppError::new("ARTICLE_SAVE_REQUIRED","请先保存文章"))?;to_value(a.images.iter().map(|u|self.cached_article_image_local(account,u)).collect::<AppResult<Vec<_>>>()?)},
            ArticleAction::Image{id}=>{let (m,bytes)=self.article_image_contents_local(account,&id)?;Ok(serde_json::json!({"image":m,"hex":hex::encode(&*bytes)}))},
            ArticleAction::History{publisher,cursor}=>{self.ensure_account(account)?;let lease=self.browse_lease(request)?;let page=self.provider.article_history(account,&publisher,cursor.as_deref(),&lease.cancel)?;lease.check()?;to_value(page)},
            ArticleAction::List{before}=>to_value(self.articles_local(account,before)?),
            ArticleAction::Get{id}=>to_value(self.article_local(account,&id)?),
            ArticleAction::Fetch{url}=>to_value(self.fetch_article_local(account,&url,request)?),
            ArticleAction::Confirm{id,revision,save}=>to_value(self.confirm_article_local(account,&id,revision,save)?),
            ArticleAction::Delete{id,revision}=>{self.ensure_account(account)?;let item=self.article_local(account,&id)?;self.article_editable(account,&item.record)?;if item.revision!=revision{return Err(AppError::new("DOCUMENT_CONFLICT","文章已更新，请刷新"));}self.repository.library_delete(account,&id,revision)?;Ok(serde_json::json!({"deleted":true}))},
            ArticleAction::CaptureLinks{capture_id,page}=>{
                let rows=self.source_capture_page_local(account,&capture_id,page)?;
                let mut urls=std::collections::BTreeMap::new();
                for row in rows.rows {
                    for text in [row.message.text.as_deref(),row.message.metadata.as_deref()].into_iter().flatten() {
                        if let Ok(Some(card))=werc_domain::message_card::parse(text) {
                            fn visit(card:&werc_domain::message_card::MessageCard,out:&mut std::collections::BTreeMap<String,String>){
                                use werc_domain::message_card::MessageCard;
                                match card {
                                    MessageCard::Link{title,url:Some(url),..}|MessageCard::AppShare{title,url:Some(url),..}=>{if let Ok(url)=canonical_url(url){out.insert(url,title.clone());}},
                                    MessageCard::Forward{entries,..}=>visit_entries(entries,out),_=>{}
                                }
                            }
                            fn visit_entries(entries:&[werc_domain::message_card::RecordEntry],out:&mut std::collections::BTreeMap<String,String>){for e in entries{if let Some(c)=&e.detail{visit(c,out)}if let Some(n)=&e.nested{visit_entries(&n.entries,out)}}}
                            visit(&card,&mut urls);
                        }
                    }
                }
                Ok(serde_json::json!({"data":urls.into_iter().map(|(url,title)|serde_json::json!({"url":url,"title":title})).collect::<Vec<_>>(),"next_page":rows.next_page,"source":"selected_capture","complete_remote_history":false}))
            }
        }.map_err(|_|AppError::internal())
    }
    pub fn export_articles_local(
        &self,
        account: &str,
        ids: &[String],
        format: ArticleExportFormat,
        request: &str,
    ) -> AppResult<ArticleExport> {
        self.ensure_account(account)?;
        let lease = self.browse_lease(request)?;
        if ids.is_empty()
            || ids.len() > 500
            || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        {
            return Err(AppError::new("INVALID_ARGUMENT", "请选择1至500篇不同文章"));
        }
        let mut rows = Vec::new();
        let mut size = 0usize;
        for id in ids {
            lease.check()?;
            let item = self.article_local(account, id)?;
            let article = item
                .record
                .saved
                .ok_or_else(|| AppError::new("ARTICLE_SAVE_REQUIRED", "请先保存所选文章的预览"))?;
            size += article.text.len();
            if size > 64 * 1024 * 1024 {
                return Err(AppError::new("EXPORT_TOO_LARGE", "请分批导出文章"));
            }
            rows.push(article);
        }
        let mut external_images=rows.iter().map(|a|a.images.len()).sum();
        let bytes = match format {
            ArticleExportFormat::Zip=>{let (bytes,missing)=self.article_bundle(account,&rows,&lease)?;external_images=missing;bytes},
            ArticleExportFormat::Json => serde_json::to_vec_pretty(&rows).map_err(|_| error())?,
            ArticleExportFormat::Md => rows
                .iter()
                .map(|a| {
                    format!(
                        "# {}\n\n{} · {}\n\n来源：{}\n\n{}\n\n{}\n",
                        a.title,
                        a.author,
                        a.published,
                        a.url,
                        a.text,
                        a.images
                            .iter()
                            .map(|u| format!("图片链接：{u}\n"))
                            .collect::<String>()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n---\n\n")
                .into_bytes(),
            ArticleExportFormat::Html => {
                let mut text = String::from(
                    "<!doctype html><meta charset=utf-8><meta http-equiv=Content-Security-Policy content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>WeRC 公众号文章</title><style>body{max-width:70ch;margin:2em auto;padding:1em;font:18px/1.7 sans-serif}pre{white-space:pre-wrap;overflow-wrap:anywhere}article{border-bottom:1px solid #bbb;padding-bottom:2em}</style>",
                );
                for a in &rows {
                    text.push_str(&format!("<article><h1>{}</h1><p>{} · {}</p><p>来源：{}</p><pre>{}</pre><details><summary>原文图片链接（未嵌入文件）</summary>{}</details></article>",escaped(&a.title),escaped(&a.author),escaped(&a.published),escaped(&a.url),escaped(&a.text),a.images.iter().map(|u|format!("<p>{}</p>",escaped(u))).collect::<String>()));
                }
                text.into_bytes()
            }
            ArticleExportFormat::Xlsx => {
                let mut book = rust_xlsxwriter::Workbook::new();
                let sheet = book.add_worksheet();
                for (col, label) in [
                    "标题",
                    "公众号/作者",
                    "发布日期",
                    "来源链接",
                    "正文分段",
                    "段号",
                ]
                .iter()
                .enumerate()
                {
                    sheet
                        .write_string(0, col as u16, *label)
                        .map_err(|_| error())?;
                }
                let mut line = 1u32;
                for a in &rows {
                    let chars = a.text.chars().collect::<Vec<_>>();
                    let parts = if chars.is_empty() {
                        vec![&[][..]]
                    } else {
                        chars.chunks(16000).collect::<Vec<_>>()
                    };
                    for (index, part) in parts.into_iter().enumerate() {
                        for (col, text) in [
                            &a.title,
                            &a.author,
                            &a.published,
                            &a.url,
                            &part.iter().collect::<String>(),
                            &(index + 1).to_string(),
                        ]
                        .iter()
                        .enumerate()
                        {
                            sheet
                                .write_string(line, col as u16, text.as_str())
                                .map_err(|_| error())?;
                        }
                        line += 1;
                    }
                }
                let images=book.add_worksheet();images.set_name("图片链接").map_err(|_|error())?;
                for (c,label) in ["标题","原文链接","图片序号","图片链接"].iter().enumerate(){images.write_string(0,c as u16,*label).map_err(|_|error())?;}
                let mut line=1;
                for a in &rows {for (i,url) in a.images.iter().enumerate(){
                    for (c,text) in [a.title.as_str(),a.url.as_str(),&(i+1).to_string(),url.as_str()].iter().enumerate(){images.write_string(line,c as u16,*text).map_err(|_|error())?;}line+=1;
                }}
                book.save_to_buffer().map_err(|_| error())?
            }
        };
        lease.check()?;
        Ok(ArticleExport {
            bytes,
            rows: rows.len(),
            external_images,
        })
    }
}
