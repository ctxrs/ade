use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use ctx_desktop_ipc::{
    DesktopWebviewRecoveryAction, DesktopWebviewRecoveryAutomationSnapshot,
    DesktopWebviewRecoveryDaemonHealth, DesktopWebviewRecoveryHeartbeatReq,
    DesktopWebviewRecoveryIncident, DesktopWebviewRecoverySuppressionReason,
    DesktopWebviewRecoveryTriggerKind, DesktopWebviewRecoveryWindowAutomationSnapshot,
    DesktopWebviewSurface,
};

use super::policy::{
    HEARTBEAT_CONFIRMATION_MS, HEARTBEAT_TIMEOUT_MS, STARTUP_GRACE_MS, decide_recovery_action,
    now_ms, prune_recent_incidents,
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
pub(crate) struct DesktopWebviewRecoveryController {
    inner: Mutex<DesktopWebviewRecoveryState>,
}

#[derive(Debug, Default)]
struct DesktopWebviewRecoveryState {
    windows: HashMap<String, RecoveryWindowState>,
}

#[derive(Debug, Clone)]
struct RecoveryWindowState {
    created_at_ms: u64,
    daemon_health: DesktopWebviewRecoveryDaemonHealth,
    exists: bool,
    last_suppressed_at_ms: Option<u64>,
    last_heartbeat_at_ms: Option<u64>,
    pending_heartbeat_timeout: bool,
    recent_incident_timestamps_ms: VecDeque<u64>,
    recovery_in_progress: bool,
    route: String,
    stale_detected_at_ms: Option<u64>,
    startup_completed_at_ms: Option<u64>,
    surface: DesktopWebviewSurface,
    window_label: String,
}

impl RecoveryWindowState {
    fn new(
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

fn normalize_route(route: &str) -> String {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

impl DesktopWebviewRecoveryController {
    pub(super) fn register_window(
        &self,
        window_label: &str,
        surface: DesktopWebviewSurface,
        route: &str,
        created_at_ms: u64,
    ) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let entry = guard
            .windows
            .entry(window_label.to_string())
            .or_insert_with(|| {
                RecoveryWindowState::new(window_label, surface, route, created_at_ms)
            });
        entry.window_label = window_label.to_string();
        entry.surface = surface;
        entry.route = normalize_route(route);
        entry.created_at_ms = created_at_ms;
        entry.daemon_health = DesktopWebviewRecoveryDaemonHealth::Unknown;
        entry.exists = true;
        entry.last_suppressed_at_ms = None;
        entry.last_heartbeat_at_ms = None;
        entry.pending_heartbeat_timeout = false;
        entry.recovery_in_progress = false;
        entry.stale_detected_at_ms = None;
        entry.startup_completed_at_ms = None;
    }

    pub(super) fn update_route(
        &self,
        window_label: &str,
        surface: DesktopWebviewSurface,
        route: &str,
    ) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let entry = guard
            .windows
            .entry(window_label.to_string())
            .or_insert_with(|| RecoveryWindowState::new(window_label, surface, route, now_ms()));
        entry.window_label = window_label.to_string();
        entry.surface = surface;
        entry.route = normalize_route(route);
        entry.exists = true;
    }

    pub(super) fn note_window_destroyed(&self, window_label: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let Some(entry) = guard.windows.get_mut(window_label) else {
            return;
        };
        entry.exists = false;
        entry.pending_heartbeat_timeout = false;
        entry.recovery_in_progress = false;
        entry.stale_detected_at_ms = None;
    }

    pub(super) fn note_heartbeat(
        &self,
        window_label: &str,
        req: &DesktopWebviewRecoveryHeartbeatReq,
    ) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let now = now_ms();
        let entry = guard
            .windows
            .entry(window_label.to_string())
            .or_insert_with(|| {
                RecoveryWindowState::new(
                    window_label,
                    DesktopWebviewSurface::Unknown,
                    &req.route,
                    now,
                )
            });
        entry.route = normalize_route(&req.route);
        entry.exists = true;
        entry.last_suppressed_at_ms = None;
        entry.last_heartbeat_at_ms = Some(now);
        entry.pending_heartbeat_timeout = false;
        entry.stale_detected_at_ms = None;
        if req.startup_ready {
            entry.startup_completed_at_ms = Some(now);
        }
    }

    pub(crate) fn rearm_heartbeat_detection(&self, window_label: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let Some(entry) = guard.windows.get_mut(window_label) else {
            return;
        };
        if entry.recovery_in_progress {
            return;
        }
        entry.last_suppressed_at_ms = None;
        entry.pending_heartbeat_timeout = false;
        entry.stale_detected_at_ms = None;
    }

    pub(super) fn evaluate_heartbeat_timeout(
        &self,
        window_label: &str,
        now_ms: u64,
    ) -> HeartbeatTimeoutEvaluation {
        let Ok(mut guard) = self.inner.lock() else {
            return HeartbeatTimeoutEvaluation::Skip;
        };
        let Some(entry) = guard.windows.get_mut(window_label) else {
            return HeartbeatTimeoutEvaluation::Skip;
        };
        if !entry.exists || entry.recovery_in_progress || entry.pending_heartbeat_timeout {
            return HeartbeatTimeoutEvaluation::Skip;
        }
        if let Some(last_suppressed_at_ms) = entry.last_suppressed_at_ms {
            if now_ms.saturating_sub(last_suppressed_at_ms) < HEARTBEAT_TIMEOUT_MS {
                return HeartbeatTimeoutEvaluation::Skip;
            }
            entry.last_suppressed_at_ms = None;
        }
        if entry.startup_completed_at_ms.is_none()
            && now_ms.saturating_sub(entry.created_at_ms) < STARTUP_GRACE_MS
        {
            return HeartbeatTimeoutEvaluation::Skip;
        }
        let last_heartbeat_at_ms = entry.last_heartbeat_at_ms.unwrap_or(entry.created_at_ms);
        if now_ms.saturating_sub(last_heartbeat_at_ms) < HEARTBEAT_TIMEOUT_MS {
            entry.pending_heartbeat_timeout = false;
            entry.stale_detected_at_ms = None;
            return HeartbeatTimeoutEvaluation::Skip;
        }
        let stale_detected_at_ms = match entry.stale_detected_at_ms {
            Some(value) => value,
            None => {
                entry.stale_detected_at_ms = Some(now_ms);
                return HeartbeatTimeoutEvaluation::AwaitConfirmation;
            }
        };
        if now_ms.saturating_sub(stale_detected_at_ms) < HEARTBEAT_CONFIRMATION_MS {
            return HeartbeatTimeoutEvaluation::AwaitConfirmation;
        }
        entry.pending_heartbeat_timeout = true;
        HeartbeatTimeoutEvaluation::Ready
    }

    pub(super) fn prepare_incident(
        &self,
        window_label: &str,
        trigger_kind: DesktopWebviewRecoveryTriggerKind,
        daemon_health: DesktopWebviewRecoveryDaemonHealth,
        suppression_reason: Option<DesktopWebviewRecoverySuppressionReason>,
        created_at_ms: u64,
        force: bool,
    ) -> Option<PreparedRecoveryIncident> {
        let Ok(mut guard) = self.inner.lock() else {
            return None;
        };
        let entry = guard.windows.get_mut(window_label)?;
        if !entry.exists || entry.recovery_in_progress {
            return None;
        }
        if trigger_kind == DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout && !force {
            if !entry.pending_heartbeat_timeout {
                return None;
            }
            if entry.startup_completed_at_ms.is_none()
                && created_at_ms.saturating_sub(entry.created_at_ms) < STARTUP_GRACE_MS
            {
                entry.pending_heartbeat_timeout = false;
                entry.stale_detected_at_ms = None;
                return None;
            }
            let last_heartbeat_at_ms = entry.last_heartbeat_at_ms.unwrap_or(entry.created_at_ms);
            if created_at_ms.saturating_sub(last_heartbeat_at_ms) < HEARTBEAT_TIMEOUT_MS {
                entry.pending_heartbeat_timeout = false;
                entry.stale_detected_at_ms = None;
                return None;
            }
        }
        let recent_incident_count =
            prune_recent_incidents(&mut entry.recent_incident_timestamps_ms, created_at_ms);
        let decision_daemon_health = match trigger_kind {
            DesktopWebviewRecoveryTriggerKind::NativeProcessTermination => {
                DesktopWebviewRecoveryDaemonHealth::Ok
            }
            DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout => daemon_health,
        };
        let action = decide_recovery_action(
            recent_incident_count,
            decision_daemon_health,
            suppression_reason,
        );
        entry.daemon_health = daemon_health;
        if trigger_kind == DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout {
            entry.pending_heartbeat_timeout = true;
        }
        if action != DesktopWebviewRecoveryAction::Noop {
            entry.recovery_in_progress = true;
            entry.recent_incident_timestamps_ms.push_back(created_at_ms);
        }
        let incident = DesktopWebviewRecoveryIncident {
            incident_id: uuid::Uuid::new_v4().to_string(),
            window_label: entry.window_label.clone(),
            window_surface: entry.surface,
            route: entry.route.clone(),
            trigger_kind,
            action,
            daemon_health,
            suppression_reason,
            created_at_ms,
        };
        Some(PreparedRecoveryIncident { incident, action })
    }

    pub(super) fn finish_recovery_action(
        &self,
        window_label: &str,
        trigger_kind: DesktopWebviewRecoveryTriggerKind,
        action: DesktopWebviewRecoveryAction,
        finished_at_ms: u64,
    ) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let Some(entry) = guard.windows.get_mut(window_label) else {
            return;
        };
        entry.recovery_in_progress = false;
        entry.pending_heartbeat_timeout = false;
        match (trigger_kind, action) {
            (
                DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
                DesktopWebviewRecoveryAction::Noop,
            ) => {
                entry.last_suppressed_at_ms = Some(finished_at_ms);
                entry.stale_detected_at_ms = None;
            }
            (_, DesktopWebviewRecoveryAction::Noop) => {}
            (_, _) => {
                entry.created_at_ms = finished_at_ms;
                entry.last_suppressed_at_ms = None;
                entry.last_heartbeat_at_ms = None;
                entry.stale_detected_at_ms = None;
                entry.startup_completed_at_ms = None;
            }
        }
    }

    pub(super) fn fail_recovery_action(&self, window_label: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let Some(entry) = guard.windows.get_mut(window_label) else {
            return;
        };
        entry.recovery_in_progress = false;
        entry.pending_heartbeat_timeout = false;
    }

    pub(super) fn current_window_labels(&self) -> Vec<String> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        guard
            .windows
            .iter()
            .filter_map(|(label, state)| state.exists.then(|| label.clone()))
            .collect()
    }

    pub(super) fn automation_snapshot(&self) -> DesktopWebviewRecoveryAutomationSnapshot {
        let Ok(mut guard) = self.inner.lock() else {
            return DesktopWebviewRecoveryAutomationSnapshot {
                windows: Vec::new(),
                pending_incident_count: 0,
            };
        };
        let now = now_ms();
        let mut windows = Vec::with_capacity(guard.windows.len());
        for state in guard.windows.values_mut() {
            let recent_incident_count =
                prune_recent_incidents(&mut state.recent_incident_timestamps_ms, now);
            windows.push(DesktopWebviewRecoveryWindowAutomationSnapshot {
                window_label: state.window_label.clone(),
                window_surface: state.surface,
                route: state.route.clone(),
                last_heartbeat_at_ms: state.last_heartbeat_at_ms,
                startup_completed_at_ms: state.startup_completed_at_ms,
                recovery_in_progress: state.recovery_in_progress,
                consecutive_recovery_count: recent_incident_count as u32,
                pending_heartbeat_timeout: state.pending_heartbeat_timeout,
                daemon_health: state.daemon_health,
                recent_incident_count: recent_incident_count as u32,
            });
        }
        windows.sort_by(|left, right| left.window_label.cmp(&right.window_label));
        let pending_incident_count = windows
            .iter()
            .filter(|window| window.pending_heartbeat_timeout || window.recovery_in_progress)
            .count() as u32;
        DesktopWebviewRecoveryAutomationSnapshot {
            windows,
            pending_incident_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_timeout_requires_confirmation_after_startup_grace() {
        let controller = DesktopWebviewRecoveryController::default();
        controller.register_window("main", DesktopWebviewSurface::Main, "/", 0);

        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", STARTUP_GRACE_MS - 1),
            HeartbeatTimeoutEvaluation::Skip
        );
        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", STARTUP_GRACE_MS),
            HeartbeatTimeoutEvaluation::AwaitConfirmation
        );
        assert_eq!(
            controller.evaluate_heartbeat_timeout(
                "main",
                STARTUP_GRACE_MS + HEARTBEAT_CONFIRMATION_MS - 1
            ),
            HeartbeatTimeoutEvaluation::AwaitConfirmation
        );
        assert_eq!(
            controller
                .evaluate_heartbeat_timeout("main", STARTUP_GRACE_MS + HEARTBEAT_CONFIRMATION_MS),
            HeartbeatTimeoutEvaluation::Ready
        );
    }

    #[test]
    fn successful_recovery_resets_heartbeat_tracking() {
        let controller = DesktopWebviewRecoveryController::default();
        controller.register_window("main", DesktopWebviewSurface::Main, "/", 0);
        controller.note_heartbeat(
            "main",
            &DesktopWebviewRecoveryHeartbeatReq {
                route: "/".to_string(),
                document_visible: true,
                window_focused: true,
                startup_ready: true,
            },
        );

        controller.finish_recovery_action(
            "main",
            DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
            DesktopWebviewRecoveryAction::Reload,
            30_000,
        );

        let snapshot = controller.automation_snapshot();
        let window = snapshot
            .windows
            .iter()
            .find(|window| window.window_label == "main")
            .expect("main window snapshot");
        assert_eq!(window.last_heartbeat_at_ms, None);
        assert_eq!(window.startup_completed_at_ms, None);

        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", 30_000 + STARTUP_GRACE_MS - 1),
            HeartbeatTimeoutEvaluation::Skip
        );
        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", 30_000 + STARTUP_GRACE_MS),
            HeartbeatTimeoutEvaluation::AwaitConfirmation
        );
    }

    #[test]
    fn focus_rearms_suppressed_heartbeat_detection() {
        let controller = DesktopWebviewRecoveryController::default();
        controller.register_window("main", DesktopWebviewSurface::Main, "/", 0);

        let prepared = controller
            .prepare_incident(
                "main",
                DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
                DesktopWebviewRecoveryDaemonHealth::Down,
                Some(DesktopWebviewRecoverySuppressionReason::DaemonDown),
                30_000,
                true,
            )
            .expect("incident");
        assert_eq!(prepared.action, DesktopWebviewRecoveryAction::Noop);

        controller.finish_recovery_action(
            "main",
            DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
            DesktopWebviewRecoveryAction::Noop,
            30_000,
        );
        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", 30_001),
            HeartbeatTimeoutEvaluation::Skip
        );

        controller.rearm_heartbeat_detection("main");
        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", 30_001),
            HeartbeatTimeoutEvaluation::AwaitConfirmation
        );
    }

    #[test]
    fn late_heartbeat_cancels_reserved_timeout_before_recovery() {
        let controller = DesktopWebviewRecoveryController::default();
        controller.register_window("main", DesktopWebviewSurface::Main, "/", 0);

        assert_eq!(
            controller.evaluate_heartbeat_timeout("main", STARTUP_GRACE_MS),
            HeartbeatTimeoutEvaluation::AwaitConfirmation
        );
        assert_eq!(
            controller
                .evaluate_heartbeat_timeout("main", STARTUP_GRACE_MS + HEARTBEAT_CONFIRMATION_MS),
            HeartbeatTimeoutEvaluation::Ready
        );

        let snapshot = controller.automation_snapshot();
        let window = snapshot
            .windows
            .iter()
            .find(|window| window.window_label == "main")
            .expect("main window snapshot");
        assert!(window.pending_heartbeat_timeout);

        controller.note_heartbeat(
            "main",
            &DesktopWebviewRecoveryHeartbeatReq {
                route: "/".to_string(),
                document_visible: true,
                window_focused: true,
                startup_ready: true,
            },
        );

        let prepared = controller.prepare_incident(
            "main",
            DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
            DesktopWebviewRecoveryDaemonHealth::Ok,
            None,
            STARTUP_GRACE_MS + HEARTBEAT_CONFIRMATION_MS + 1,
            false,
        );
        assert!(prepared.is_none());

        let snapshot = controller.automation_snapshot();
        let window = snapshot
            .windows
            .iter()
            .find(|window| window.window_label == "main")
            .expect("main window snapshot");
        assert!(!window.pending_heartbeat_timeout);
    }

    #[test]
    fn failed_recovery_preserves_heartbeat_tracking() {
        let controller = DesktopWebviewRecoveryController::default();
        controller.register_window("main", DesktopWebviewSurface::Main, "/", 0);
        controller.note_heartbeat(
            "main",
            &DesktopWebviewRecoveryHeartbeatReq {
                route: "/".to_string(),
                document_visible: true,
                window_focused: true,
                startup_ready: true,
            },
        );

        let prepared = controller
            .prepare_incident(
                "main",
                DesktopWebviewRecoveryTriggerKind::HeartbeatTimeout,
                DesktopWebviewRecoveryDaemonHealth::Ok,
                None,
                now_ms(),
                true,
            )
            .expect("incident");
        assert_eq!(prepared.action, DesktopWebviewRecoveryAction::Reload);

        controller.fail_recovery_action("main");

        let snapshot = controller.automation_snapshot();
        let window = snapshot
            .windows
            .iter()
            .find(|window| window.window_label == "main")
            .expect("main window snapshot");
        assert!(window.last_heartbeat_at_ms.is_some());
        assert!(window.startup_completed_at_ms.is_some());
        assert!(!window.recovery_in_progress);
        assert!(!window.pending_heartbeat_timeout);
    }
}
