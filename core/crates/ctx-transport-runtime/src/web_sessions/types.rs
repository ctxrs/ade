use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSessionStatus {
    Running,
    Closed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionInfo {
    pub id: String,
    pub kind: String,
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub status: WebSessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub url: String,
    pub viewport: WebSessionViewport,
    pub fps: u32,
    pub viewers: u32,
    pub stream_path: String,
    pub stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionCreateRequest {
    pub url: String,
    pub viewport: Option<WebSessionViewport>,
    pub fps: Option<u32>,
    pub work_dir: Option<PathBuf>,
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub node_bin: PathBuf,
    pub worker_path: PathBuf,
    pub node_modules_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionRunRequest {
    pub code: Option<String>,
    pub script_path: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionRunResponse {
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
