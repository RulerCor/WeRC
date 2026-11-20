use serde::{Deserialize, Serialize};
#[derive(Serialize)]
pub struct AsrStatus {
    pub available: bool,
    pub models: Vec<String>,
    pub engine: &'static str,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Segment {
    pub from_ms: u64,
    pub until_ms: u64,
    pub text: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub model: String,
    pub language: String,
    pub text: String,
    pub segments: Vec<Segment>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioSource {
    Archived {
        capture_id: String,
        page: u64,
        row: usize,
    },
    Uploaded {
        asset_id: String,
    },
    Received {
        position: i64,
    },
    SavedVoice {
        file_id: String,
    },
}

impl Transcript {
    pub fn srt(&self) -> String {
        fn time(ms: u64) -> String {
            format!(
                "{:02}:{:02}:{:02},{:03}",
                ms / 3600000,
                ms / 60000 % 60,
                ms / 1000 % 60,
                ms % 1000
            )
        }
        self.segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{}\n{} --> {}\n{}\n\n",
                    i + 1,
                    time(s.from_ms),
                    time(s.until_ms),
                    s.text
                )
            })
            .collect()
    }
}
