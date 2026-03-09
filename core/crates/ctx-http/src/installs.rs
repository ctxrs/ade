use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

pub type InstallId = Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstallTarget {
    Host,
    Container,
    LinuxAarch64,
    LinuxX8664,
}

impl InstallTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Container => "container",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::LinuxX8664 => "linux-x86_64",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallEventLevel {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallErrorCode {
    InvalidTarget,
    UnsupportedTarget,
    DownloadFailed,
    ChecksumMismatch,
    CommandFailed,
    Timeout,
    MatrixMismatch,
    HealthCheckFailed,
    RegistryWriteFailed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgressEvent {
    pub install_id: InstallId,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<InstallTarget>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<InstallErrorCode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStateKind {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallInfo {
    pub install_id: InstallId,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<InstallTarget>,
    pub state: InstallStateKind,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<InstallErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<InstallProgressEvent>,
}

pub struct InstallState {
    pub provider_id: String,
    pub target: Option<InstallTarget>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub state: InstallStateKind,
    pub error: Option<String>,
    pub error_code: Option<InstallErrorCode>,
    pub events: VecDeque<InstallProgressEvent>,
    pub mirrors: HashSet<InstallId>,
    pub info_event_override: Option<InstallProgressEvent>,
    pub info_event_override_until: Option<DateTime<Utc>>,
    pub tx: broadcast::Sender<InstallProgressEvent>,
}

impl InstallState {
    pub fn new(provider_id: String, target: Option<InstallTarget>) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            provider_id,
            target,
            started_at: Utc::now(),
            finished_at: None,
            state: InstallStateKind::Running,
            error: None,
            error_code: None,
            events: VecDeque::with_capacity(256),
            mirrors: HashSet::new(),
            info_event_override: None,
            info_event_override_until: None,
            tx,
        }
    }

    pub fn info(&self, install_id: InstallId) -> InstallInfo {
        self.build_info(install_id, self.events.back().cloned())
    }

    pub fn polling_info(&self, install_id: InstallId) -> InstallInfo {
        let current_last_event = self.events.back().cloned();
        let override_visible = matches!(self.state, InstallStateKind::Running)
            && self
                .info_event_override_until
                .is_some_and(|visible_until| Utc::now() <= visible_until)
            && self.should_expose_info_event_override(current_last_event.as_ref());
        let last_event = if override_visible {
            self.info_event_override
                .clone()
                .or_else(|| current_last_event.clone())
        } else {
            current_last_event
        };
        self.build_info(install_id, last_event)
    }

    fn should_expose_info_event_override(
        &self,
        current_last_event: Option<&InstallProgressEvent>,
    ) -> bool {
        if self.info_event_override.is_none() {
            return false;
        }
        let Some(current_last_event) = current_last_event else {
            return true;
        };
        current_last_event.message.starts_with("Prerequisite ")
            || matches!(current_last_event.stage.as_str(), "start" | "prerequisites")
    }

    fn build_info(
        &self,
        install_id: InstallId,
        last_event: Option<InstallProgressEvent>,
    ) -> InstallInfo {
        InstallInfo {
            install_id,
            provider_id: self.provider_id.clone(),
            target: self.target,
            state: self.state,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error: self.error.clone(),
            error_code: self.error_code,
            last_event,
        }
    }
}

pub fn truncate_for_storage(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut out = s.chars().take(max_len).collect::<String>();
    out.push('…');
    out
}
