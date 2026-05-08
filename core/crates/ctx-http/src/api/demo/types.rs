use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct SeedTranscriptTurnReq {
    pub(crate) user: String,
    pub(crate) assistant: String,
    #[serde(default)]
    pub(crate) context_window: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeedTranscriptReq {
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) task_title: Option<String>,
    #[serde(default)]
    pub(crate) append: bool,
    #[serde(default = "default_seed_transcript_refresh")]
    pub(crate) refresh: bool,
    #[serde(default)]
    pub(crate) materialize_tail_turns: Option<usize>,
    pub(crate) turns: Vec<SeedTranscriptTurnReq>,
}

fn default_seed_transcript_refresh() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub(crate) struct SeedTranscriptResp {
    pub(crate) session_id: String,
    pub(crate) seeded_turns: usize,
    pub(crate) seeded_messages: usize,
    pub(crate) seeded_events: usize,
}
