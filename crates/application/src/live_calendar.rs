use crate::Application;
use serde::Serialize;
use werc_domain::{AppError, AppResult, live::*};

#[derive(Serialize)]
pub struct CalendarPage {
    pub days: Vec<LiveCalendarDay>,
    pub counts: Vec<u64>,
    pub source_count: usize,
    pub source_snapshot_atomic: bool,
    pub expires_in_seconds: u64,
}
impl Application {
    pub fn live_calendar_local(
        &self,
        account: &str,
        parent_view_id: &str,
        selection: &LiveCalendarSelection,
        request_id: &str,
    ) -> AppResult<CalendarPage> {
        if !selection.valid() {
            return Err(AppError::new(
                "LIVE_CALENDAR_INVALID",
                "日历需要连续的最多31天及准确会话账号",
            ));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let (binding, sources) =
            self.with_search_parent(account, parent_view_id, |binding, _, sources, _| {
                Ok((binding.to_owned(), sources.to_vec()))
            })?;
        let keys = self.live_keys_local(account)?;
        let mut counts = vec![0u64; selection.days.len()];
        let start = std::time::Instant::now();
        for source in &sources {
            lease.check()?;
            crate::live_multi::budget(start)?;
            let key = crate::keyring::key_for(&keys, &source.salt)?;
            let window = match self.provider.read_live_calendar(
                account,
                &source.database,
                &key,
                &source.fingerprint,
                selection,
                &lease.cancel,
            ) {
                Ok(window) => window,
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                Err(e) => return Err(e),
            };
            if !window.valid(selection) {
                return Err(AppError::new("LIVE_PROTOCOL_INVALID", "日历统计结果无效"));
            }
            for (total, count) in counts.iter_mut().zip(window.counts) {
                *total = total
                    .checked_add(count)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or_else(|| AppError::new("LIVE_CALENDAR_LIMIT", "计数超过可显示范围"))?;
            }
        }
        lease.check()?;
        crate::live_multi::budget(start)?;
        self.with_search_parent(account, parent_view_id, |current, parent, _, _| {
            if current != binding {
                return Err(AppError::new("CURSOR_STALE", "日历读取期间来源发生变化"));
            }
            let expires = parent.check(current, parent_view_id)?;
            lease.check()?;
            Ok(CalendarPage {
                days: selection.days.clone(),
                counts,
                source_count: sources.len(),
                source_snapshot_atomic: sources.len() == 1,
                expires_in_seconds: expires,
            })
        })
    }
}
