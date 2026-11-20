//! Local archival schedules; no Bot messaging or business rules.
use crate::{Application, capture_tools::CaptureExportOptions};
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPlanSpec {
    pub title: String,
    pub source_capture: String,
    pub refresh_source: bool,
    pub formats: Vec<String>,
    pub directory: String,
    pub options: CaptureExportOptions,
    pub interval_seconds: i64,
    pub lookback_seconds: Option<i64>,
    pub since_last_success: bool,
    pub enabled: bool,
    pub next_run: i64,
    pub stop_at: Option<i64>,
    pub max_runs: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ExportPlan {
    pub spec: ExportPlanSpec,
    pub run_count: u64,
    pub success_count: u64,
    pub last_success_until: Option<i64>,
    pub active_run: Option<String>,
    pub deleted: bool,
}
#[derive(Serialize)]
pub struct PlanItem {
    pub id: String,
    pub revision: i64,
    pub plan: ExportPlan,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ExportArtifact {
    pub format: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub rows: u64,
    pub published: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ExportRun {
    pub plan_id: String,
    pub title: String,
    pub spec: ExportPlanSpec,
    pub state: String,
    pub from: Option<i64>,
    pub until: i64,
    pub capture_id: String,
    pub artifacts: Vec<ExportArtifact>,
    pub cancel_requested: bool,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub error: Option<AppError>,
}
#[derive(Serialize)]
pub struct RunItem {
    pub id: String,
    pub revision: i64,
    pub run: ExportRun,
}
fn conflict() -> AppError {
    AppError::new("DOCUMENT_CONFLICT", "计划已更新，请刷新后重试")
}
fn running(state: &str) -> bool {
    matches!(state, "queued" | "reading" | "exporting")
}
fn validate(s: &ExportPlanSpec) -> AppResult<()> {
    crate::capture_tools::interchange::validate_columns(&s.options.xlsx_columns)?;
    if !s.options.filter.valid() {
        return Err(AppError::new("INVALID_ARGUMENT", "计划筛选条件无效"));
    }
    let path = std::path::Path::new(&s.directory);
    if s.title.trim().is_empty()
        || s.title.len() > 256
        || s.formats.is_empty()
        || s.formats.len() > 12
        || s.formats
            .iter()
            .any(|f| crate::capture_tools::capture_export_extension(f).is_none())
        || s.formats
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != s.formats.len()
        || s.directory.is_empty()
        || s.directory.len() > 512
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        || s.directory.contains([':', '<', '>', '"', '|', '?', '*'])
        || s.directory.chars().any(char::is_control)
        || !(60..=31536000).contains(&s.interval_seconds)
        || s.lookback_seconds.is_some_and(|v| v <= 0 || v > 315360000)
        || s.next_run < 0
        || s.stop_at.is_some_and(|v| v < 0)
        || s.max_runs == Some(0)
        || (s.since_last_success && !s.refresh_source)
        || s.options.filter.from.is_some()
        || s.options.filter.until.is_some()
        || s.options.replacements.len() > 64
        || s.options
            .replacements
            .iter()
            .any(|(a, b)| a.is_empty() || a.len() > 1024 || b.len() > 1024)
        || s.options
            .text_template
            .as_ref()
            .is_some_and(|v| v.len() > 16384)
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "计划名称、格式、相对目录、时间或选项无效",
        ));
    }
    if s.options
        .filter
        .conversations
        .iter()
        .any(|v| !werc_domain::live::valid_live_username(v))
    {
        return Err(AppError::new("INVALID_ARGUMENT", "会话账号无效"));
    }
    Ok(())
}
impl Application {
    pub fn export_plans_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<PlanItem>, Option<i64>)> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "export_plan", before, 20)?;
        let data = page
            .data
            .iter()
            .map(|h| {
                let (h, plan) = self.open_document(account, &h.id, "export_plan")?;
                Ok(PlanItem {
                    id: h.id,
                    revision: h.revision,
                    plan,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok((data, page.next_cursor))
    }
    pub fn save_export_plan_local(
        &self,
        account: &str,
        id: Option<&str>,
        revision: i64,
        spec: ExportPlanSpec,
    ) -> AppResult<PlanItem> {
        if !(0..i64::MAX).contains(&revision) {
            return Err(AppError::new("INVALID_ARGUMENT", "计划版本无效"));
        }
        validate(&spec)?;
        let capture = self.source_capture_status_local(account, &spec.source_capture)?;
        if spec.refresh_source && capture.imported {
            return Err(AppError::new(
                "ARCHIVE_READ_ONLY",
                "导入归档只能按已有副本导出",
            ));
        }
        let (id, mut plan) = if let Some(id) = id {
            let (h, plan): (_, ExportPlan) = self.open_document(account, id, "export_plan")?;
            if h.revision != revision {
                return Err(conflict());
            }
            if plan.active_run.is_some() || plan.deleted {
                return Err(AppError::new("BUSY", "请先取消或完成运行中的计划"));
            }
            (id.to_owned(), plan)
        } else {
            if revision != 0 {
                return Err(conflict());
            }
            (
                uuid::Uuid::new_v4().to_string(),
                ExportPlan {
                    spec: spec.clone(),
                    run_count: 0,
                    success_count: 0,
                    last_success_until: None,
                    active_run: None,
                    deleted: false,
                },
            )
        };
        // An incremental watermark belongs to a particular source and conversation scope.
        if plan.spec.source_capture != spec.source_capture
            || plan.spec.options.filter != spec.options.filter
            || plan.spec.lookback_seconds != spec.lookback_seconds
            || plan.spec.since_last_success != spec.since_last_success
            || plan.spec.refresh_source != spec.refresh_source
        {
            plan.last_success_until = None;
        }
        plan.spec = spec;
        let doc = self.seal_document(account, &id, "export_plan", revision + 1, &plan)?;
        self.repository.library_save(&[doc], None)?;
        Ok(PlanItem {
            id,
            revision: revision + 1,
            plan,
        })
    }
    pub fn delete_export_plan_local(
        &self,
        account: &str,
        id: &str,
        revision: i64,
    ) -> AppResult<()> {
        let (h, mut plan): (_, ExportPlan) = self.open_document(account, id, "export_plan")?;
        if h.revision != revision {
            return Err(conflict());
        }
        if plan.active_run.is_some() {
            return Err(AppError::new("BUSY", "先取消或完成当前运行"));
        }
        plan.deleted = true;
        plan.spec.enabled = false;
        self.repository.library_save(
            &[self.seal_document(account, id, "export_plan", h.revision + 1, &plan)?],
            None,
        )
    }
    pub fn begin_export_run_local(
        &self,
        account: &str,
        id: &str,
        manual: bool,
    ) -> AppResult<RunItem> {
        let (h, mut plan): (_, ExportPlan) = self.open_document(account, id, "export_plan")?;
        let now = chrono::Utc::now().timestamp();
        if plan.deleted || plan.active_run.is_some() {
            return Err(AppError::new("BUSY", "计划已删除或正在运行"));
        }
        if !manual
            && (!plan.spec.enabled
                || plan.spec.next_run > now
                || plan.spec.stop_at.is_some_and(|v| now >= v)
                || plan.spec.max_runs.is_some_and(|n| plan.run_count >= n))
        {
            return Err(AppError::new("NOT_DUE", "计划尚未到期或已结束"));
        }
        let id_run = uuid::Uuid::new_v4().to_string();
        let from = if plan.spec.since_last_success {
            plan.last_success_until.or_else(|| {
                plan.spec
                    .lookback_seconds
                    .map(|v| now.saturating_sub(v).max(0))
            })
        } else {
            plan.spec
                .lookback_seconds
                .map(|v| now.saturating_sub(v).max(0))
        };
        let empty = from.is_some_and(|v| v >= now);
        let run = ExportRun {
            plan_id: id.into(),
            title: plan.spec.title.clone(),
            spec: plan.spec.clone(),
            state: if empty { "no_new_data" } else { "queued" }.into(),
            from,
            until: now,
            capture_id: if plan.spec.refresh_source {
                format!("{id_run}-capture")
            } else {
                plan.spec.source_capture.clone()
            },
            artifacts: vec![],
            cancel_requested: false,
            started_at: chrono::Utc::now().to_rfc3339(),
            ended_at: empty.then(|| chrono::Utc::now().to_rfc3339()),
            error: None,
        };
        plan.active_run = (!empty).then(|| id_run.clone());
        plan.run_count += 1;
        plan.spec.next_run = now.saturating_add(plan.spec.interval_seconds);
        self.repository.library_save(
            &[
                self.seal_document(account, id, "export_plan", h.revision + 1, &plan)?,
                self.seal_document(account, &id_run, "export_run", 1, &run)?,
            ],
            None,
        )?;
        Ok(RunItem {
            id: id_run,
            revision: 1,
            run,
        })
    }
    pub fn export_run_local(&self, account: &str, id: &str) -> AppResult<RunItem> {
        let (h, run) = self.open_document(account, id, "export_run")?;
        Ok(RunItem {
            id: h.id,
            revision: h.revision,
            run,
        })
    }
    pub fn export_runs_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<RunItem>, Option<i64>)> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "export_run", before, 20)?;
        Ok((
            page.data
                .iter()
                .map(|h| self.export_run_local(account, &h.id))
                .collect::<AppResult<_>>()?,
            page.next_cursor,
        ))
    }
    fn update_export_run(
        &self,
        account: &str,
        id: &str,
        change: impl Fn(&mut ExportRun) -> AppResult<()>,
    ) -> AppResult<RunItem> {
        for _ in 0..3 {
            let mut item = self.export_run_local(account, id)?;
            change(&mut item.run)?;
            let doc =
                self.seal_document(account, id, "export_run", item.revision + 1, &item.run)?;
            match self.repository.library_save(&[doc], None) {
                Ok(()) => {
                    item.revision += 1;
                    return Ok(item);
                }
                Err(e) if e.code == "DOCUMENT_CONFLICT" => continue,
                Err(e) => return Err(e),
            }
        }
        Err(conflict())
    }
    pub fn export_run_phase_local(
        &self,
        account: &str,
        id: &str,
        phase: &str,
    ) -> AppResult<RunItem> {
        if !matches!(phase, "reading" | "exporting") {
            return Err(AppError::new("INVALID_ARGUMENT", "运行阶段无效"));
        }
        self.update_export_run(account, id, |run| {
            if !running(&run.state) || run.cancel_requested {
                return Err(AppError::new("QUERY_CANCELED", "任务已停止"));
            }
            run.state = phase.into();
            Ok(())
        })
    }
    pub fn record_export_artifact_local(
        &self,
        account: &str,
        id: &str,
        artifact: ExportArtifact,
    ) -> AppResult<RunItem> {
        self.update_export_run(account, id, |run| {
            if run.state != "exporting" || run.cancel_requested {
                return Err(AppError::new("QUERY_CANCELED", "任务已停止"));
            }
            if let Some(old) = run
                .artifacts
                .iter_mut()
                .find(|a| a.format == artifact.format)
            {
                *old = artifact.clone();
            } else {
                run.artifacts.push(artifact.clone());
            }
            Ok(())
        })
    }
    pub fn cancel_export_run_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.update_export_run(account, id, |run| {
            if running(&run.state) {
                run.cancel_requested = true;
            }
            Ok(())
        })?;
        self.cancel_browse_local(id)
    }
    pub fn finish_export_run_local(
        &self,
        account: &str,
        id: &str,
        state: &str,
        error: Option<AppError>,
    ) -> AppResult<RunItem> {
        if !matches!(state, "succeeded" | "failed" | "canceled" | "interrupted") {
            return Err(AppError::new("INVALID_ARGUMENT", "运行结果无效"));
        }
        for _ in 0..3 {
            let mut item = self.export_run_local(account, id)?;
            let (h, mut plan): (_, ExportPlan) =
                self.open_document(account, &item.run.plan_id, "export_plan")?;
            if plan.active_run.as_deref() != Some(id) {
                return Ok(item);
            }
            if state == "succeeded"
                && (item.run.spec.formats.iter().any(|f| {
                    !item
                        .run
                        .artifacts
                        .iter()
                        .any(|a| a.format == *f && a.published)
                }))
            {
                return Err(AppError::new("EXPORT_INCOMPLETE", "产物尚未全部保存"));
            }
            item.run.state = if item.run.cancel_requested {
                "canceled"
            } else {
                state
            }
            .into();
            item.run.error = error.clone();
            item.run.ended_at = Some(chrono::Utc::now().to_rfc3339());
            plan.active_run = None;
            if item.run.state == "succeeded" {
                plan.success_count += 1;
                plan.last_success_until = Some(item.run.until);
            }
            let result = self.repository.library_save(
                &[
                    self.seal_document(
                        account,
                        &item.run.plan_id,
                        "export_plan",
                        h.revision + 1,
                        &plan,
                    )?,
                    self.seal_document(account, id, "export_run", item.revision + 1, &item.run)?,
                ],
                None,
            );
            match result {
                Ok(()) => {
                    item.revision += 1;
                    return Ok(item);
                }
                Err(e) if e.code == "DOCUMENT_CONFLICT" => continue,
                Err(e) => return Err(e),
            }
        }
        Err(conflict())
    }
    pub fn resume_export_run_local(&self, account: &str, id: &str) -> AppResult<RunItem> {
        let mut item = self.export_run_local(account, id)?;
        let (h, mut plan): (_, ExportPlan) =
            self.open_document(account, &item.run.plan_id, "export_plan")?;
        if plan.active_run.is_some()
            || plan.deleted
            || !matches!(
                item.run.state.as_str(),
                "failed" | "interrupted" | "canceled"
            )
        {
            return Err(AppError::new("BUSY", "此任务不能继续"));
        }
        item.run.state = "queued".into();
        item.run.cancel_requested = false;
        item.run.error = None;
        item.run.ended_at = None;
        plan.active_run = Some(id.into());
        // Consume any pre-start cancellation belonging to the previous attempt.
        if let Ok(lease) = self.browse_lease(id) {
            drop(lease);
        }
        self.repository.library_save(
            &[
                self.seal_document(
                    account,
                    &item.run.plan_id,
                    "export_plan",
                    h.revision + 1,
                    &plan,
                )?,
                self.seal_document(account, id, "export_run", item.revision + 1, &item.run)?,
            ],
            None,
        )?;
        item.revision += 1;
        Ok(item)
    }
}
