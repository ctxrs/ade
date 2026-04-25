use std::collections::{HashMap, VecDeque};

use ctx_desktop_ipc::{
    DesktopWebviewRecoveryAction, DesktopWebviewRecoveryDaemonHealth,
    DesktopWebviewRecoveryIncident, DesktopWebviewSurface,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeartbeatTimeoutEvaluation {
    Skip,
    AwaitConfirmation,
    Ready,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedRecoveryIncident {
    pub incident: DesktopWebviewRecoveryIncident,
    pub action: DesktopWebviewRecoveryAction,
}

#[derive(Debug, Default)]
pub(super) struct DesktopWebviewRecoveryState {
    pub(super) windows: HashMap<String, RecoveryWindowState>,
}

#[derive(Debug, Clone)]
pub(super) struct RecoveryWindowState {
    pub(super) created_at_ms: u64,
    pub(super) daemon_health: DesktopWebviewRecoveryDaemonHealth,
    pub(super) exists: bool,
    pub(super) last_suppressed_at_ms: Option<u64>,
    pub(super) last_heartbeat_at_ms: Option<u64>,
    pub(super) pending_heartbeat_timeout: bool,
    pub(super) recent_incident_timestamps_ms: VecDeque<u64>,
    pub(super) recovery_in_progress: bool,
    pub(super) route: String,
    pub(super) stale_detected_at_ms: Option<u64>,
    pub(super) startup_completed_at_ms: Option<u64>,
    pub(super) surface: DesktopWebviewSurface,
    pub(super) window_label: String,
}

impl RecoveryWindowState {
    pub(super) fn new(
        window_label: &str,
        surface: DesktopWebviewSurface,
        route: &str,
        created_at_ms: u64,
    ) -> Self {
        Self {
            created_at_ms,
            daemon_health: DesktopWebviewRecoveryDaemonHealth::Unknown,
            exists: true,
            last_suppressed_at_ms: None,
            last_heartbeat_at_ms: None,
            pending_heartbeat_timeout: false,
            recent_incident_timestamps_ms: VecDeque::new(),
            recovery_in_progress: false,
            route: normalize_route(route),
            stale_detected_at_ms: None,
            startup_completed_at_ms: None,
            surface,
            window_label: window_label.to_string(),
        }
    }
}

pub(super) fn normalize_route(route: &str) -> String {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}
