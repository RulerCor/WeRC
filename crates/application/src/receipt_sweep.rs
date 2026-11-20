//! Bounded receipt queries. Never renew approval or submit a second attempt.
use crate::Application;
use serde::Serialize;
use werc_domain::{
    AppResult, Principal,
    outbox::{SendState, SendStatus},
};

#[derive(Serialize)]
pub struct ReceiptSweep {
    pub next_cursor: Option<i64>,
    pub examined: usize,
    pub updated: usize,
    pub unresolved: usize,
    pub last_error_code: Option<String>,
}

impl Application {
    pub fn reconcile_send(&self, p: &Principal, account: &str, id: &str) -> AppResult<SendStatus> {
        // A receipt query has no external send side effect, and can be requested by a read client.
        let status = self.send_status(p, account, id)?;
        if !matches!(status.state, SendState::Unknown | SendState::ServerAccepted) {
            return Ok(status);
        }
        self.reconcile_send_local(account, id)
    }

    pub fn reconcile_receipt_page_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<ReceiptSweep> {
        self.ensure_current_connection(account)?;
        // Give the sender another turn between native receipt queries.
        let page = self.repository.pending_receipt_page(account, before, 1)?;
        let mut progress = ReceiptSweep {
            next_cursor: page.next_cursor,
            examined: 0,
            updated: 0,
            unresolved: 0,
            last_error_code: None,
        };
        for previous in page.data {
            progress.examined += 1;
            match self.reconcile_send_local(account, &previous.metadata.id) {
                Ok(current) => {
                    if current.state != previous.state || current.evidence != previous.evidence {
                        progress.updated += 1;
                    }
                    if matches!(
                        current.state,
                        SendState::Unknown | SendState::ServerAccepted
                    ) {
                        progress.unresolved += 1;
                    }
                }
                Err(e) => {
                    progress.unresolved += 1;
                    progress.last_error_code = Some(e.code.clone());
                    if matches!(e.code.as_str(), "BUSY" | "NATIVE_DISCONNECTED") {
                        // Retry this page on the next pass, rather than skipping untouched tasks.
                        progress.next_cursor = before;
                        break;
                    }
                    if e.code == "CURRENT_CONNECTION_REQUIRED" {
                        return Err(e);
                    }
                }
            }
        }
        Ok(progress)
    }
}
