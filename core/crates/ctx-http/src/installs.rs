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
        let synthetic_start_event = if matches!(self.state, InstallStateKind::Running) {
            self.synthetic_polling_start_event(install_id)
        } else {
            None
        };
        let override_visible = matches!(self.state, InstallStateKind::Running)
            && self
                .info_event_override_until
                .is_some_and(|visible_until| Utc::now() <= visible_until)
            && self.should_expose_info_event_override(current_last_event.as_ref());
        let last_event = if override_visible {
            self.info_event_override
                .clone()
                .or_else(|| current_last_event.clone())
                .or(synthetic_start_event)
        } else {
            current_last_event.or(synthetic_start_event)
        };
        self.build_info(install_id, last_event)
    }

    fn synthetic_polling_start_event(&self, install_id: InstallId) -> Option<InstallProgressEvent> {
        if self.events.back().is_some() {
            return None;
        }
        Some(InstallProgressEvent {
            install_id,
            provider_id: self.provider_id.clone(),
            target: self.target,
            at: self.started_at,
            stage: "start".to_string(),
            message: "Install started".to_string(),
            level: InstallEventLevel::Info,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: None,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_info_surfaces_synthetic_start_event_for_running_installs_without_progress() {
        let install_id = InstallId::new_v4();
        let state = InstallState::new("acp-crp-bridge".to_string(), Some(InstallTarget::Container));

        let info = state.polling_info(install_id);

        assert!(matches!(info.state, InstallStateKind::Running));
        assert_eq!(
            info.last_event.as_ref().map(|event| event.stage.as_str()),
            Some("start")
        );
        assert_eq!(
            info.last_event.as_ref().map(|event| event.message.as_str()),
            Some("Install started")
        );
    }

    #[test]
    fn polling_info_prefers_real_progress_events_over_synthetic_start() {
        let install_id = InstallId::new_v4();
        let mut state =
            InstallState::new("acp-crp-bridge".to_string(), Some(InstallTarget::Container));
        state.events.push_back(InstallProgressEvent {
            install_id,
            provider_id: "acp-crp-bridge".to_string(),
            target: Some(InstallTarget::Container),
            at: Utc::now(),
            stage: "download".to_string(),
            message: "Downloading bridge".to_string(),
            level: InstallEventLevel::Info,
            bytes: Some(32),
            total_bytes: Some(64),
            attempt: Some(1),
            error_code: None,
        });

        let info = state.polling_info(install_id);

        assert_eq!(
            info.last_event.as_ref().map(|event| event.stage.as_str()),
            Some("download")
        );
    }
}
