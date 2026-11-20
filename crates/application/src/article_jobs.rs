//! Local article acquisition plans. Read operations resume; publication is explicit.
use crate::{
    Application,
    articles::{ArticleItem, ArticleRecord},
    export_plans::ExportArtifact,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, articles::canonical_url};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArticlePlanSpec {
    pub title: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub cache_images: bool,
    pub interval_seconds: i64,
    pub next_run: i64,
    pub enabled: bool,
    pub formats: Vec<String>,
    pub directory: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ArticlePlan {
    pub spec: ArticlePlanSpec,
    pub active_run: Option<String>,
    pub deleted: bool,
    pub run_count: u64,
}
#[derive(Serialize)]
pub struct PlanItem {
    pub id: String,
    pub revision: i64,
    pub plan: ArticlePlan,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ArticleResult {
    pub url: String,
    pub id: Option<String>,
    pub revision: Option<i64>,
    pub error: Option<AppError>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ArticleRun {
    pub plan_id: String,
    pub spec: ArticlePlanSpec,
    pub state: String,
    pub results: Vec<ArticleResult>,
    #[serde(default)]
    pub discovered_publishers: usize,
    #[serde(default)]
    pub discovery_errors: Vec<AppError>,
    #[serde(default)]
    pub image_errors: Vec<ArticleImageFailure>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub archived: bool,
    pub artifacts: Vec<ExportArtifact>,
    pub error: Option<AppError>,
}
#[derive(Clone,Serialize,Deserialize)]
pub struct ArticleImageFailure {pub article_id:String,pub index:usize,pub error:AppError}
#[derive(Serialize)]
pub struct RunItem {
    pub id: String,
    pub revision: i64,
    pub run: ArticleRun,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum JobAction {
    Plans {
        before: Option<i64>,
    },
    Runs {
        before: Option<i64>,
    },
    GetRun {
        id: String,
    },
    Save {
        id: Option<String>,
        revision: i64,
        spec: ArticlePlanSpec,
    },
    Delete {
        id: String,
        revision: i64,
    },
    Start {
        id: String,
    },
    Step {
        id: String,
    },
    Cancel {
        id: String,
    },
    Confirm {
        id: String,
        revision: i64,
        save: bool,
    },
    Resume {
        id: String,
    },
    Archive {
        id: String,
        revision: i64,
    },
}
impl JobAction {
    pub fn writes(&self) -> bool {
        !matches!(
            self,
            Self::Plans { .. } | Self::Runs { .. } | Self::GetRun { .. }
        )
    }
}
fn conflict() -> AppError {
    AppError::new("DOCUMENT_CONFLICT", "文章任务已更新，请刷新")
}
fn active(state: &str) -> bool {
    matches!(state, "queued" | "reading" | "pending" | "exporting")
}
fn validate(spec: &mut ArticlePlanSpec) -> AppResult<()> {
    if spec.title.trim().is_empty()
        || spec.title.len() > 256
        || (spec.urls.is_empty() && spec.publishers.is_empty())
        || spec.publishers.len()>10
        || spec.publishers.iter().any(|s| !werc_domain::article_history::publisher_valid(s))
        || spec.publishers.iter().collect::<std::collections::HashSet<_>>().len()!=spec.publishers.len()
        || spec.urls.len() > 100
        || !(3600..=31536000).contains(&spec.interval_seconds)
        || spec.next_run < 0
        || spec.formats.is_empty()
        || spec.formats.len() > 5
        || spec
            .formats
            .iter()
            .any(|s| !matches!(s.as_str(), "md" | "html" | "json" | "xlsx" | "zip"))
        || spec
            .formats
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != spec.formats.len()
        || spec.directory.is_empty()
        || spec.directory.len() > 256
        || std::path::Path::new(&spec.directory).is_absolute()
        || spec.directory.contains([':', '<', '>', '"', '|', '?', '*'])
        || spec.directory.chars().any(char::is_control)
        || std::path::Path::new(&spec.directory)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "请检查计划名称、文章链接、间隔、格式与相对目录",
        ));
    }
    spec.urls = spec
        .urls
        .iter()
        .map(|s| canonical_url(s))
        .collect::<AppResult<_>>()?;
    if spec
        .urls
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != spec.urls.len()
    {
        return Err(AppError::new("INVALID_ARGUMENT", "计划含有重复文章链接"));
    }
    Ok(())
}
impl Application {
    pub fn article_plans_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<PlanItem>, Option<i64>)> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "article_plan", before, 20)?;
        Ok((
            page.data
                .iter()
                .map(|h| {
                    let (h, plan) = self.open_document(account, &h.id, "article_plan")?;
                    Ok(PlanItem {
                        id: h.id,
                        revision: h.revision,
                        plan,
                    })
                })
                .collect::<AppResult<_>>()?,
            page.next_cursor,
        ))
    }
    pub fn article_run_local(&self, account: &str, id: &str) -> AppResult<RunItem> {
        let (h, run) = self.open_document(account, id, "article_run")?;
        Ok(RunItem {
            id: h.id,
            revision: h.revision,
            run,
        })
    }
    pub fn article_runs_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<RunItem>, Option<i64>)> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "article_run", before, 20)?;
        Ok((
            page.data
                .iter()
                .map(|h| self.article_run_local(account, &h.id))
                .collect::<AppResult<_>>()?,
            page.next_cursor,
        ))
    }
    pub fn save_article_plan_local(
        &self,
        account: &str,
        id: Option<&str>,
        revision: i64,
        mut spec: ArticlePlanSpec,
    ) -> AppResult<PlanItem> {
        self.ensure_account(account)?;
        validate(&mut spec)?;
        if !(0..i64::MAX).contains(&revision) {
            return Err(conflict());
        }
        let (id, mut plan) = if let Some(id) = id {
            let (h, p): (_, ArticlePlan) = self.open_document(account, id, "article_plan")?;
            if h.revision != revision {
                return Err(conflict());
            }
            if p.deleted || p.active_run.is_some() {
                return Err(AppError::new("BUSY", "计划有未结束的采集，请先完成或取消"));
            }
            (id.to_owned(), p)
        } else {
            if revision != 0 {
                return Err(conflict());
            }
            (
                uuid::Uuid::new_v4().to_string(),
                ArticlePlan {
                    spec: spec.clone(),
                    active_run: None,
                    deleted: false,
                    run_count: 0,
                },
            )
        };
        plan.spec = spec;
        self.repository.library_save(
            &[self.seal_document(account, &id, "article_plan", revision + 1, &plan)?],
            None,
        )?;
        Ok(PlanItem {
            id,
            revision: revision + 1,
            plan,
        })
    }
    pub fn begin_article_run_local(
        &self,
        account: &str,
        id: &str,
        manual: bool,
    ) -> AppResult<RunItem> {
        let (h, mut plan): (_, ArticlePlan) = self.open_document(account, id, "article_plan")?;
        let now = chrono::Utc::now().timestamp();
        if plan.deleted {
            return Err(AppError::new("NOT_FOUND", "计划已删除"));
        }
        if let Some(active) = plan.active_run {
            return self.article_run_local(account, &active);
        }
        if !manual && (!plan.spec.enabled || plan.spec.next_run > now) {
            return Err(AppError::new("NOT_DUE", "计划未到执行时间"));
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        let run = ArticleRun {
            plan_id: id.into(),
            spec: plan.spec.clone(),
            state: "queued".into(),
            results: vec![],
            discovered_publishers: 0,
            discovery_errors: vec![],
            image_errors: vec![],
            started_at: now,
            ended_at: None,
            archived: false,
            artifacts: vec![],
            error: None,
        };
        plan.active_run = Some(run_id.clone());
        plan.run_count += 1;
        plan.spec.next_run = now.saturating_add(plan.spec.interval_seconds);
        self.repository.library_save(
            &[
                self.seal_document(account, id, "article_plan", h.revision + 1, &plan)?,
                self.seal_document(account, &run_id, "article_run", 1, &run)?,
            ],
            None,
        )?;
        Ok(RunItem {
            id: run_id,
            revision: 1,
            run,
        })
    }
    pub fn step_article_run_local(&self, account: &str, id: &str) -> AppResult<RunItem> {
        let mut item = self.article_run_local(account, id)?;
        if !matches!(item.run.state.as_str(), "queued" | "reading") {
            return Ok(item);
        }
        if let Some(publisher)=item.run.spec.publishers.get(item.run.discovered_publishers) {
            let lease=self.browse_lease(id)?;
            let result=self.provider.article_history(account,publisher,None,&lease.cancel);
            lease.check()?;
            match result {
                Ok(page)=>{for row in page.data {if item.run.spec.urls.len()<100 && !item.run.spec.urls.contains(&row.url){item.run.spec.urls.push(row.url);}}},
                Err(e)=>item.run.discovery_errors.push(e),
            }
            item.run.discovered_publishers+=1;
            self.repository.library_save(&[self.seal_document(account,id,"article_run",item.revision+1,&item.run)?],None)?;
            item.revision+=1;return Ok(item);
        }
        if item.run.spec.urls.is_empty(){return self.finish_article_run_local(account,id,"failed",Some(AppError::new("ARTICLE_DISCOVERY_EMPTY","公众号未返回可读取文章，请查看采集详情")));}
        let index = item.run.results.len();
        let url = item.run.spec.urls.get(index).ok_or_else(conflict)?.clone();
        let lease = self.browse_lease(id)?;
        let fetched = self.provider.fetch_article(account, &url, &lease.cancel);
        lease.check()?;
        let mut docs = Vec::new();
        let row = match fetched {
            Ok(article) => {
                if !article.valid() {
                    return Err(AppError::new(
                        "ARTICLE_CONTENT_INVALID",
                        "文章组件返回无效正文",
                    ));
                }
                // Each run owns its preview, so another plan cannot replace a pending result.
                let article_id = format!(
                    "article-{}",
                    hex::encode(Sha256::digest(format!("{id}:{index}").as_bytes()))
                );
                let record = ArticleRecord {
                    saved: None,
                    pending: Some(article),
                    owner_run: Some(id.into()),
                };
                docs.push(self.seal_document(account, &article_id, "web_article", 1, &record)?);
                ArticleResult {
                    url,
                    id: Some(article_id),
                    revision: Some(1),
                    error: None,
                }
            }
            Err(error) => ArticleResult {
                url,
                id: None,
                revision: None,
                error: Some(error),
            },
        };
        item.run.results.push(row);
        item.run.state = if item.run.results.len() == item.run.spec.urls.len() {
            "pending"
        } else {
            "reading"
        }
        .into();
        docs.push(self.seal_document(account, id, "article_run", item.revision + 1, &item.run)?);
        lease.check()?;
        self.repository.library_save(&docs, None)?;
        item.revision += 1;
        Ok(item)
    }
    pub fn finish_article_run_local(
        &self,
        account: &str,
        id: &str,
        state: &str,
        error: Option<AppError>,
    ) -> AppResult<RunItem> {
        if !matches!(
            state,
            "succeeded" | "partial" | "failed" | "canceled" | "discarded"
        ) {
            return Err(AppError::new("INVALID_ARGUMENT", "任务结束状态无效"));
        }
        let mut item = self.article_run_local(account, id)?;
        let (h, mut plan): (_, ArticlePlan) =
            self.open_document(account, &item.run.plan_id, "article_plan")?;
        if !active(&item.run.state) {
            return Ok(item);
        }
        if plan.active_run.as_deref() != Some(id) {
            return Err(conflict());
        }
        if matches!(state, "succeeded" | "partial")
            && (item.run.state != "exporting"
                || item.run.spec.formats.iter().any(|f| {
                    !item
                        .run
                        .artifacts
                        .iter()
                        .any(|a| a.format == *f && a.published)
                }))
        {
            return Err(AppError::new("EXPORT_INCOMPLETE", "文章产物尚未全部发布"));
        }
        item.run.state = state.into();
        item.run.error = error;
        item.run.ended_at = Some(chrono::Utc::now().timestamp());
        plan.active_run = None;
        self.repository.library_save(
            &[
                self.seal_document(account, id, "article_run", item.revision + 1, &item.run)?,
                self.seal_document(
                    account,
                    &item.run.plan_id,
                    "article_plan",
                    h.revision + 1,
                    &plan,
                )?,
            ],
            None,
        )?;
        item.revision += 1;
        Ok(item)
    }
    pub fn confirm_article_run_local(
        &self,
        account: &str,
        id: &str,
        revision: i64,
        save: bool,
    ) -> AppResult<RunItem> {
        let mut item = self.article_run_local(account, id)?;
        if item.revision != revision {
            return Err(conflict());
        }
        if item.run.state != "pending" {
            return Err(AppError::new("INVALID_ARGUMENT", "采集尚未进入待确认状态"));
        }
        let mut docs = vec![];
        for row in &mut item.run.results {
            if let Some(article_id) = &row.id {
                let mut article = self.article_local(account, article_id)?;
                if Some(article.revision) != row.revision {
                    return Err(conflict());
                }
                let pending = article.record.pending.take().ok_or_else(conflict)?;
                if save {
                    article.record.saved = Some(pending);
                }
                article.revision += 1;
                row.revision = Some(article.revision);
                docs.push(self.seal_document(
                    account,
                    article_id,
                    "web_article",
                    article.revision,
                    &article.record,
                )?);
            }
        }
        let successful = item.run.results.iter().any(|r| r.id.is_some());
        item.run.state = if save && successful {
            "exporting"
        } else if save {
            "failed"
        } else {
            "discarded"
        }
        .into();
        if item.run.state != "exporting" {
            let (h, mut plan): (_, ArticlePlan) =
                self.open_document(account, &item.run.plan_id, "article_plan")?;
            if plan.active_run.as_deref() != Some(id) {
                return Err(conflict());
            }
            plan.active_run = None;
            item.run.ended_at = Some(chrono::Utc::now().timestamp());
            docs.push(self.seal_document(
                account,
                &item.run.plan_id,
                "article_plan",
                h.revision + 1,
                &plan,
            )?);
        }
        docs.push(self.seal_document(account, id, "article_run", item.revision + 1, &item.run)?);
        self.repository.library_save(&docs, None)?;
        item.revision += 1;
        Ok(item)
    }
    pub fn article_image_result_local(&self,account:&str,id:&str,article_id:&str,index:usize,error:Option<AppError>)->AppResult<()> {
        let mut item=self.article_run_local(account,id)?;
        if item.run.state!="exporting"{return Err(AppError::new("QUERY_CANCELED","文章任务已停止"));}
        if !item.run.results.iter().any(|r|r.id.as_deref()==Some(article_id)){return Err(AppError::new("INVALID_ARGUMENT","图片不属于本次采集"));}
        item.run.image_errors.retain(|e|e.article_id!=article_id||e.index!=index);
        if let Some(error)=error{item.run.image_errors.push(ArticleImageFailure{article_id:article_id.into(),index,error});}
        self.repository.library_save(&[self.seal_document(account,id,"article_run",item.revision+1,&item.run)?],None)
    }
    pub fn article_artifact_local(
        &self,
        account: &str,
        id: &str,
        artifact: ExportArtifact,
    ) -> AppResult<()> {
        let mut item = self.article_run_local(account, id)?;
        if item.run.state != "exporting" {
            return Err(AppError::new("QUERY_CANCELED", "文章任务已停止"));
        }
        item.run.artifacts.retain(|a| a.format != artifact.format);
        item.run.artifacts.push(artifact);
        self.repository.library_save(
            &[self.seal_document(account, id, "article_run", item.revision + 1, &item.run)?],
            None,
        )
    }
    pub fn article_job_action_local(
        &self,
        account: &str,
        action: JobAction,
    ) -> AppResult<serde_json::Value> {
        use serde_json::to_value;
        let value = match action {
            JobAction::Plans { before } => to_value(self.article_plans_local(account, before)?),
            JobAction::Runs { before } => to_value(self.article_runs_local(account, before)?),
            JobAction::GetRun { id } => to_value(self.article_run_local(account, &id)?),
            JobAction::Save { id, revision, spec } => {
                to_value(self.save_article_plan_local(account, id.as_deref(), revision, spec)?)
            }
            JobAction::Start { id } => to_value(self.begin_article_run_local(account, &id, true)?),
            JobAction::Step { id } => to_value(self.step_article_run_local(account, &id)?),
            JobAction::Cancel { id } => {
                self.article_run_local(account, &id)?;
                self.cancel_browse_local(&id)?;
                to_value(self.finish_article_run_local(account, &id, "canceled", None)?)
            }
            JobAction::Confirm { id, revision, save } => {
                to_value(self.confirm_article_run_local(account, &id, revision, save)?)
            }
            JobAction::Delete { id, revision } => {
                let (h, mut plan): (_, ArticlePlan) =
                    self.open_document(account, &id, "article_plan")?;
                if h.revision != revision {
                    return Err(conflict());
                }
                if plan.active_run.is_some() {
                    return Err(AppError::new("BUSY", "请先结束当前采集"));
                }
                plan.deleted = true;
                plan.spec.enabled = false;
                self.repository.library_save(
                    &[self.seal_document(account, &id, "article_plan", h.revision + 1, &plan)?],
                    None,
                )?;
                Ok(serde_json::json!({"deleted":true}))
            }
            JobAction::Resume { id } => {
                let mut item = self.article_run_local(account, &id)?;
                let (h, mut plan): (_, ArticlePlan) =
                    self.open_document(account, &item.run.plan_id, "article_plan")?;
                if active(&item.run.state) {
                    return to_value(item).map_err(|_| AppError::internal());
                }
                if plan.deleted
                    || plan.active_run.is_some()
                    || !matches!(item.run.state.as_str(), "failed" | "canceled")
                {
                    return Err(AppError::new("BUSY", "此任务不能继续"));
                }
                // Already saved results resume export. Completed reads return to confirmation.
                let saved = item
                    .run
                    .results
                    .iter()
                    .filter(|r| r.id.is_some())
                    .map(|r| {
                        let article=self.article_local(account,r.id.as_deref().unwrap())?;
                        if Some(article.revision)!=r.revision || article.record.owner_run.as_deref()!=Some(id.as_str()) { return Err(AppError::new("ARTICLE_RUN_CONTENT_CHANGED","这次采集的文章已被修改，请创建新采集任务")); }
                        Ok(article)
                    })
                    .collect::<AppResult<Vec<ArticleItem>>>()?;
                item.run.state = if !saved.is_empty()
                    && saved
                        .iter()
                        .all(|a| a.record.saved.is_some() && a.record.pending.is_none())
                {
                    "exporting"
                } else if item.run.discovered_publishers==item.run.spec.publishers.len() && item.run.results.len() == item.run.spec.urls.len() {
                    "pending"
                } else {
                    "reading"
                }
                .into();
                match self.browse_lease(&id) {
                    Ok(lease) => drop(lease),
                    Err(e) if e.code == "QUERY_CANCELED" => {}
                    Err(e) => return Err(e),
                }
                item.run.error = None;
                item.run.ended_at = None;
                plan.active_run = Some(id.clone());
                self.repository.library_save(
                    &[
                        self.seal_document(
                            account,
                            &id,
                            "article_run",
                            item.revision + 1,
                            &item.run,
                        )?,
                        self.seal_document(
                            account,
                            &item.run.plan_id,
                            "article_plan",
                            h.revision + 1,
                            &plan,
                        )?,
                    ],
                    None,
                )?;
                item.revision += 1;
                to_value(item)
            }
            JobAction::Archive { id, revision } => {
                let mut item = self.article_run_local(account, &id)?;
                if item.revision != revision {
                    return Err(conflict());
                }
                if active(&item.run.state) {
                    return Err(AppError::new("BUSY", "运行中的任务不能归档"));
                }
                item.run.archived = true;
                self.repository.library_save(
                    &[self.seal_document(
                        account,
                        &id,
                        "article_run",
                        item.revision + 1,
                        &item.run,
                    )?],
                    None,
                )?;
                item.revision += 1;
                to_value(item)
            }
        };
        value.map_err(|_| AppError::internal())
    }
}
