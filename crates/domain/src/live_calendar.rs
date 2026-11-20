use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveCalendarDay {
    pub from_timestamp: i64,
    pub until_timestamp: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveCalendarSelection {
    pub username: String,
    pub days: Vec<LiveCalendarDay>,
}
impl LiveCalendarSelection {
    pub fn valid(&self) -> bool {
        valid_live_username(&self.username)
            && !self.days.is_empty()
            && self.days.len() <= 31
            && self.days.iter().all(|d| {
                d.from_timestamp >= 0
                    && d.until_timestamp > d.from_timestamp
                    && d.until_timestamp <= 253402300800
                    && d.until_timestamp - d.from_timestamp <= 172800
            })
            && self
                .days
                .windows(2)
                .all(|w| w[0].until_timestamp == w[1].from_timestamp)
    }
}
impl Drop for LiveCalendarSelection {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveCalendarWindow {
    pub counts: Vec<u64>,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveCalendarWindow {
    pub fn valid(&self, selection: &LiveCalendarSelection) -> bool {
        selection.valid()
            && self.read_only
            && self.readonly_shm
            && self.counts.len() == selection.days.len()
            && self.counts.iter().all(|n| *n <= 9_007_199_254_740_991)
    }
}
