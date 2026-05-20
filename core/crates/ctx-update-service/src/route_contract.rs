use ctx_observability::logs;
use serde::{Deserialize, Serialize};

use crate::{ManagedDaemonAutoUpdateStatus, UpdateDrainState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRouteErrorKind {
    BadRequest,
    BadGateway,
    Internal,
}

#[derive(Debug, Clone)]
pub struct UpdateRouteError {
    kind: UpdateRouteErrorKind,
    message: String,
}

impl UpdateRouteError {
    pub fn bad_request(error: impl std::fmt::Display) -> Self {
        Self::new(UpdateRouteErrorKind::BadRequest, error)
    }

    pub fn bad_gateway(error: impl std::fmt::Display) -> Self {
        Self::new(UpdateRouteErrorKind::BadGateway, error)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(UpdateRouteErrorKind::Internal, error)
    }

    fn new(kind: UpdateRouteErrorKind, error: impl std::fmt::Display) -> Self {
        Self {
            kind,
            message: logs::redact_sensitive(&error.to_string()),
        }
    }

    pub fn kind(&self) -> UpdateRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveTurnRecord {
    pub workspace_id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub turn_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonTurnActivitySummary {
    pub idle: bool,
    pub active_turn_count: usize,
    pub queued_turn_count: usize,
    pub running_turn_count: usize,
    pub scanned_workspace_count: usize,
    pub turns: Vec<ActiveTurnRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_drain: Option<UpdateDrainState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonSandboxWorkActivitySummary {
    pub active: bool,
    pub active_sandbox_turn_count: usize,
    pub queued_sandbox_turn_count: usize,
    pub running_sandbox_turn_count: usize,
    pub running_container_backed_terminal: bool,
    pub running_workspace_container_count: usize,
    pub runtime_operation_count: usize,
    pub prewarm_artifact_operation_count: usize,
    pub scanned_workspace_count: usize,
    pub turns: Vec<ActiveTurnRecord>,
}

#[derive(Debug, Serialize)]
pub struct UpdateCheckSnapshot {
    pub channel: String,
    pub base_url: String,
    pub platform: Option<String>,
    pub current_version: String,
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_supported_version: Option<String>,
    pub platform_supported: bool,
    pub in_place_update_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_place_update_reason: Option<String>,
    pub update_available: bool,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub manifest: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct UpdateActivitySnapshot {
    pub activity: DaemonTurnActivitySummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_daemon_auto_update: Option<ManagedDaemonAutoUpdateStatus>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadAppImageUpdateRequest {
    #[serde(default)]
    channel: Option<String>,
}

impl DownloadAppImageUpdateRequest {
    pub fn new(channel: Option<String>) -> Self {
        Self { channel }
    }

    pub fn channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }
}

#[derive(Debug, Serialize)]
pub struct DownloadAppImageUpdateResult {
    pub downloaded_path: String,
    pub can_apply_in_place: bool,
}

#[derive(Debug, Deserialize)]
pub struct ApplyAppImageUpdateRequest {
    confirm: bool,
    #[serde(default)]
    channel: Option<String>,
}

impl ApplyAppImageUpdateRequest {
    pub fn new(confirm: bool, channel: Option<String>) -> Self {
        Self { confirm, channel }
    }

    pub fn confirm(&self) -> bool {
        self.confirm
    }

    pub fn channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }
}

#[derive(Debug, Serialize)]
pub struct ApplyAppImageUpdateResult {
    pub applied: bool,
    pub target_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct BeginUpdateDrainRouteRequest {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    owner: Option<String>,
}

impl BeginUpdateDrainRouteRequest {
    pub fn new(confirm: bool, reason: Option<String>, owner: Option<String>) -> Self {
        Self {
            confirm,
            reason,
            owner,
        }
    }

    pub fn confirm(&self) -> bool {
        self.confirm
    }

    pub fn into_reason_owner(self) -> (Option<String>, Option<String>) {
        (self.reason, self.owner)
    }
}

#[derive(Debug, Serialize)]
pub struct BeginUpdateDrainRouteResult {
    pub acquired: bool,
    pub activity: DaemonTurnActivitySummary,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseUpdateDrainRouteRequest {
    #[serde(default)]
    confirm: bool,
}

impl ReleaseUpdateDrainRouteRequest {
    pub fn new(confirm: bool) -> Self {
        Self { confirm }
    }

    pub fn confirm(&self) -> bool {
        self.confirm
    }
}

#[derive(Debug, Serialize)]
pub struct ReleaseUpdateDrainRouteResult {
    pub released: bool,
}

#[derive(Debug, Deserialize)]
pub struct ShutdownDaemonRouteRequest {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(skip)]
    supplied_shutdown_token: Option<String>,
}

impl ShutdownDaemonRouteRequest {
    pub fn new(confirm: bool, reason: Option<String>) -> Self {
        Self {
            confirm,
            reason,
            supplied_shutdown_token: None,
        }
    }

    pub fn with_supplied_shutdown_token(mut self, token: Option<String>) -> Self {
        self.supplied_shutdown_token = token;
        self
    }

    pub fn confirm(&self) -> bool {
        self.confirm
    }

    pub fn reason(self) -> Option<String> {
        self.reason
    }

    pub fn supplied_shutdown_token(&self) -> Option<&str> {
        self.supplied_shutdown_token.as_deref()
    }
}

#[derive(Debug, Serialize)]
pub struct ShutdownDaemonRouteResult {
    pub accepted: bool,
    pub activity: DaemonTurnActivitySummary,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MaintenanceRouteErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MaintenanceRouteError {
    kind: MaintenanceRouteErrorKind,
    message: String,
}

impl MaintenanceRouteError {
    pub fn kind(&self) -> MaintenanceRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Conflict,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Forbidden,
            message: message.into(),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Internal,
            message: logs::redact_sensitive(&error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_route_errors_redact_sensitive_messages() {
        let error = UpdateRouteError::internal("CTX_MCP_TOKEN=env-secret failed");

        assert_eq!(error.kind(), UpdateRouteErrorKind::Internal);
        assert!(!error.message().contains("env-secret"));
    }

    #[test]
    fn shutdown_token_is_not_json_deserializable() {
        let request: ShutdownDaemonRouteRequest = serde_json::from_value(serde_json::json!({
            "confirm": true,
            "reason": "test",
            "supplied_shutdown_token": "body-token"
        }))
        .expect("request");

        assert!(request.confirm());
        assert_eq!(request.supplied_shutdown_token(), None);
        let request = request.with_supplied_shutdown_token(Some("header-token".to_string()));
        assert_eq!(request.supplied_shutdown_token(), Some("header-token"));
    }

    #[test]
    fn activity_json_omits_absent_optional_fields() {
        let summary = DaemonTurnActivitySummary {
            idle: false,
            active_turn_count: 1,
            queued_turn_count: 0,
            running_turn_count: 1,
            scanned_workspace_count: 2,
            turns: vec![ActiveTurnRecord {
                workspace_id: "workspace".to_string(),
                session_id: "session".to_string(),
                run_id: None,
                turn_id: "turn".to_string(),
                status: "running".to_string(),
            }],
            update_drain: None,
        };

        let value = serde_json::to_value(summary).expect("json");

        assert!(value.get("update_drain").is_none());
        assert!(value["turns"][0].get("run_id").is_some());
        assert!(value["turns"][0]["run_id"].is_null());
    }

    #[test]
    fn update_activity_json_omits_absent_managed_auto_update() {
        let snapshot = UpdateActivitySnapshot {
            activity: DaemonTurnActivitySummary {
                idle: true,
                active_turn_count: 0,
                queued_turn_count: 0,
                running_turn_count: 0,
                scanned_workspace_count: 0,
                turns: Vec::new(),
                update_drain: None,
            },
            managed_daemon_auto_update: None,
        };

        let value = serde_json::to_value(snapshot).expect("json");

        assert!(value.get("managed_daemon_auto_update").is_none());
        assert!(value.get("activity").is_some());
    }
}
