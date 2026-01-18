use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartWorkerRequest {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub repo: RepoSpec,
    pub base_commit_sha: Option<String>,
    pub diff_debounce_ms: Option<u64>,
    pub ttl_seconds: Option<u64>,
    pub snapshot_ttl_seconds: Option<u64>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartWorkerResponse {
    pub worker_id: String,
    pub state: WorkerState,
    pub ssh: Option<SshInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub state: WorkerState,
    pub base_commit_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_diff_at: Option<DateTime<Utc>>,
    pub ssh: Option<SshInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_log_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegistration {
    pub worker_id: String,
    pub agent_endpoint: Option<String>,
    pub ssh: Option<SshInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_log_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffArtifact {
    pub worker_id: String,
    pub base_commit_sha: String,
    pub head_commit_sha: String,
    pub generated_at: DateTime<Utc>,
    pub patch: String,
    pub changed_files: Vec<String>,
    pub file_count: i64,
    pub line_additions: i64,
    pub line_deletions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPatchResponse {
    pub worker_id: String,
    pub base_commit_sha: String,
    pub head_commit_sha: String,
    pub generated_at: DateTime<Utc>,
    pub patch: String,
    pub changed_files: Vec<String>,
    pub file_count: i64,
    pub line_additions: i64,
    pub line_deletions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOpenRequest {
    pub terminal_id: String,
    pub shell: String,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalControlMessage {
    Open {
        terminal_id: String,
        shell: String,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    },
    Close {
        terminal_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepoSpec {
    Local { path: String },
    Git { url: String, reference: String },
    Archive { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerState {
    Starting,
    Running,
    Paused,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayMessage {
    Init {
        session_id: String,
        provider_id: String,
        model_id: Option<String>,
        env: HashMap<String, String>,
        workdir: Option<String>,
    },
    Acp {
        session_id: String,
        payload: String,
    },
    Close {
        session_id: String,
    },
}

pub fn new_worker_id() -> String {
    Uuid::new_v4().to_string()
}
