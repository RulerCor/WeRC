use super::*;
use sha2::{Digest, Sha256};
fn job_id(id: &str, model: &str, language: &str) -> String {
    format!(
        "{}:asr",
        hex::encode(Sha256::digest(
            format!("{id}\0{model}\0{language}").as_bytes()
        ))
    )
}
impl Application {
    pub fn capture_transcript_progress_local(
        &self,
        account: &str,
        id: &str,
        model: &str,
        language: &str,
    ) -> AppResult<MediaProgress> {
        self.source_capture_status_local(account, id)?;
        match self.open_document(account, &job_id(id, model, language), "capture_asr_job") {
            Ok((_, p)) => Ok(p),
            Err(e) if e.code == "NOT_FOUND" => Ok(MediaProgress::default()),
            Err(e) => Err(e),
        }
    }
    pub fn step_capture_transcripts_local(
        &self,
        account: &str,
        id: &str,
        model: &str,
        language: &str,
        restart: bool,
        request_id: &str,
    ) -> AppResult<MediaProgress> {
        let job_id = job_id(id, model, language);
        let (revision, mut progress) =
            match self.open_document::<MediaProgress>(account, &job_id, "capture_asr_job") {
                Ok((h, p)) => (h.revision, p),
                Err(e) if e.code == "NOT_FOUND" => (0, MediaProgress::default()),
                Err(e) => return Err(e),
            };
        if restart {
            progress = MediaProgress::default();
        }
        let status = self.source_capture_status_local(account, id)?;
        if progress.page < status.pages {
            let rows = self.capture_media_page_local(account, id, progress.page)?;
            if let Some(row) = rows.get(progress.row) {
                if row.saved.as_ref().is_some_and(|m| {
                    m.kind == crate::conversation_media::ConversationMediaKind::Voice
                }) {
                    match self.transcribe_local(
                        account,
                        &werc_domain::asr::AudioSource::Archived {
                            capture_id: id.into(),
                            page: progress.page,
                            row: progress.row,
                        },
                        model,
                        language,
                        request_id,
                    ) {
                        Ok(_) => progress.saved += 1,
                        Err(e)
                            if matches!(
                                e.code.as_str(),
                                "QUERY_CANCELED"
                                    | "ASR_UNAVAILABLE"
                                    | "ASR_START_FAILED"
                                    | "INVALID_ARGUMENT"
                            ) =>
                        {
                            return Err(e);
                        }
                        Err(e) => {
                            progress.failed += 1;
                            progress.recent_errors.push(format!(
                                "第{}页第{}条：{} ({})",
                                progress.page + 1,
                                progress.row + 1,
                                e.message,
                                e.code
                            ));
                            if progress.recent_errors.len() > 20 {
                                progress.recent_errors.remove(0);
                            }
                        }
                    }
                } else {
                    progress.skipped += 1;
                }
                progress.row += 1;
            }
            if progress.row >= rows.len() {
                progress.row = 0;
                progress.page += 1;
            }
        }
        progress.complete = progress.page >= status.pages;
        self.repository.library_save(
            &[self.seal_document(account, &job_id, "capture_asr_job", revision + 1, &progress)?],
            None,
        )?;
        Ok(progress)
    }
}
