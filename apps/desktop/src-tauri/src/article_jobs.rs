use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use werc_application::{
    article_jobs::RunItem, articles::ArticleExportFormat, export_plans::ExportArtifact,
};
use werc_domain::AppResult;
fn export(app: &Application, account: &str, item: RunItem, downloads: &Path) -> AppResult<()> {
    let ids = item
        .run
        .results
        .iter()
        .filter_map(|r| r.id.clone())
        .collect::<Vec<_>>();
    if item.run.spec.cache_images {
        for id in &ids {
            let article=app.article_local(account,id)?;
            if let Some(saved)=article.record.saved {
                for index in 0..saved.images.len(){
                    if app.article_run_local(account,&item.id)?.run.state!="exporting"{return Err(AppError::new("QUERY_CANCELED","文章任务已停止"));}
                    let result=app.cache_article_image_local(account,id,index,&item.id);
                    if let Err(e)=&result {if e.code=="QUERY_CANCELED"{return Err(e.clone());}}
                    app.article_image_result_local(account,&item.id,id,index,result.err())?;
                }
            }
        }
    }
    for row in &item.run.results {
        if let Some(id)=&row.id {
            let article=app.article_local(account,id)?;
            if Some(article.revision)!=row.revision || article.record.owner_run.as_deref()!=Some(item.id.as_str()) {return Err(AppError::new("ARTICLE_RUN_CONTENT_CHANGED","采集文章已变化，请新建任务"));}
        }
    }
    for format in &item.run.spec.formats {
        let current = app.article_run_local(account, &item.id)?;
        if current.run.state != "exporting" {
            return Err(AppError::new("QUERY_CANCELED", "文章导出已取消"));
        }
        let (relative, path) =
            export_jobs::output_path(downloads, &item.run.spec.directory, &item.id, format)?;
        if path.exists() {
            let old = current
                .run
                .artifacts
                .iter()
                .find(|a| a.path == relative && a.format == *format)
                .ok_or_else(|| {
                    AppError::new(
                        "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                        "目标已有文件，未覆盖",
                    )
                })?;
            if !export_jobs::matches_artifact(&path, old)? {
                return Err(AppError::new(
                    "EXPORT_ARTIFACT_CHANGED",
                    "已有文章产物被修改，未覆盖",
                ));
            }
            let mut artifact = old.clone();
            artifact.published = true;
            app.article_artifact_local(account, &item.id, artifact)?;
            continue;
        }
        let f: ArticleExportFormat =
            serde_json::from_value(serde_json::json!(format)).map_err(|_| AppError::internal())?;
        let output = app.export_articles_local(account, &ids, f, &item.id)?;
        let mut artifact = ExportArtifact {
            format: format.clone(),
            path: relative,
            sha256: hex::encode(Sha256::digest(&output.bytes)),
            size: output.bytes.len() as u64,
            rows: output.rows as u64,
            published: false,
        };
        export_file::stream_new(&path, |file| {
            use std::io::Write;
            file.write_all(&output.bytes)
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "文章文件写入失败"))?;
            app.article_artifact_local(account, &item.id, artifact.clone())
        })?;
        artifact.published = true;
        app.article_artifact_local(account, &item.id, artifact)?;
    }
    let current=app.article_run_local(account,&item.id)?;
    let state = if !current.run.image_errors.is_empty() || !item.run.discovery_errors.is_empty() || item.run.results.iter().any(|r| r.error.is_some()) {
        "partial"
    } else {
        "succeeded"
    };
    app.finish_article_run_local(account, &item.id, state, None)?;
    Ok(())
}
fn tick(app: &Application, downloads: &Path) -> AppResult<bool> {
    for account in app.accounts_local()? {
        let mut before = None;
        loop {
            let (plans, next) = app.article_plans_local(&account.id, before)?;
            for p in plans {
                if p.plan.deleted {
                    continue;
                }
                let run = if let Some(id) = p.plan.active_run {
                    app.article_run_local(&account.id, &id)?
                } else {
                    match app.begin_article_run_local(&account.id, &p.id, false) {
                        Ok(v) => v,
                        Err(e) if matches!(e.code.as_str(), "NOT_DUE" | "DOCUMENT_CONFLICT") => {
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                };
                if run.run.state == "pending" {
                    continue;
                }
                let id = run.id.clone();
                let result = match run.run.state.as_str() {
                    "queued" | "reading" => app
                        .step_article_run_local(&account.id, &id)
                        .map(|v| v.run.state == "pending"),
                    "exporting" => export(app, &account.id, run, downloads).map(|_| true),
                    _ => continue,
                };
                return match result {
                    Ok(done) => Ok(done),
                    Err(e)
                        if matches!(
                            e.code.as_str(),
                            "BUSY" | "QUERY_IN_PROGRESS" | "DOCUMENT_CONFLICT"
                        ) =>
                    {
                        Ok(false)
                    }
                    Err(e) => {
                        let state = if e.code == "QUERY_CANCELED" {
                            "canceled"
                        } else {
                            "failed"
                        };
                        app.finish_article_run_local(&account.id, &id, state, Some(e))?;
                        Ok(true)
                    }
                };
            }
            before = next;
            if before.is_none() {
                break;
            }
        }
    }
    Ok(false)
}
pub(super) fn start(
    app: Arc<Application>,
    downloads: PathBuf,
    allow_live: bool,
    handle: tauri::AppHandle,
) {
    if !allow_live {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let worker = app.clone();
            let output = downloads.clone();
            match tauri::async_runtime::spawn_blocking(move || tick(&worker, &output)).await {
                Ok(Ok(true)) => notifications::task_finished_at(&handle, "library"),
                Ok(Err(e)) => diagnostics::record(&handle, "article-task", &e),
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}
