use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

pub type InstallId = Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallEventLevel {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgressEvent {
    pub install_id: InstallId,
    pub provider_id: String,
    pub at: DateTime<Utc>,
    pub stage: String,
    pub message: String,
    pub level: InstallEventLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStateKind {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallInfo {
    pub install_id: InstallId,
    pub provider_id: String,
    pub state: InstallStateKind,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<InstallProgressEvent>,
}

pub struct InstallState {
    pub provider_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub state: InstallStateKind,
    pub error: Option<String>,
    pub events: VecDeque<InstallProgressEvent>,
    pub tx: broadcast::Sender<InstallProgressEvent>,
}

impl InstallState {
    pub fn new(provider_id: String) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            provider_id,
            started_at: Utc::now(),
            finished_at: None,
            state: InstallStateKind::Running,
            error: None,
            events: VecDeque::with_capacity(256),
            tx,
        }
    }

    pub fn info(&self, install_id: InstallId) -> InstallInfo {
        InstallInfo {
            install_id,
            provider_id: self.provider_id.clone(),
            state: self.state,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error: self.error.clone(),
            last_event: self.events.back().cloned(),
        }
    }
}

pub fn truncate_for_storage(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut out = s.chars().take(max_len).collect::<String>();
    out.push_str("…");
    out
}

