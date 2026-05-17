use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use ctx_observability::logs;
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;
use serde::{Deserialize, Serialize};

use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary,
    reconcile_running_turns_with_reason, spawn_deferred_daemon_shutdown,
    DaemonSandboxWorkActivitySummary, DaemonState, DaemonTurnActivitySummary, ExecutionHandle,
};

pub struct MaintenanceDrainPermit {
    state: Arc<DaemonState>,
    released: bool,
}

impl MaintenanceDrainPermit {
    pub async fn release(mut self) -> bool {
        if self.released {
            return false;
        }
        let released = self.state.core.update_drain.release().await;
        self.released = true;
        released
    }
}

impl Drop for MaintenanceDrainPermit {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        let state = Arc::clone(&self.state);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = state.core.update_drain.release().await;
                });
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "maintenance drain permit dropped without a tokio runtime; drain may remain active"
                );
            }
        }
    }
}

#[derive(Debug)]
pub enum BeginUpdateDrainError {
    AlreadyActive,
    ActivityUnavailable(Error),
    Busy,
}

#[derive(Debug)]
pub enum MaintenanceDrainError {
    AlreadyActive,
    ActivityUnavailable(Error),
    SandboxWorkActive,
}

#[derive(Debug)]
pub enum DaemonShutdownError {
    ActivityUnavailable(Error),
    Reconcile(Error),
}

#[derive(Debug, Deserialize)]
pub struct BeginUpdateDrainRouteRequest {
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BeginUpdateDrainRouteResult {
    pub acquired: bool,
    pub activity: DaemonTurnActivitySummary,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseUpdateDrainRouteRequest {
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct ReleaseUpdateDrainRouteResult {
    pub released: bool,
}

#[derive(Debug, Deserialize)]
pub struct ShutdownDaemonRouteRequest {
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(skip)]
    supplied_shutdown_token: Option<String>,
}

impl ShutdownDaemonRouteRequest {
    pub fn with_supplied_shutdown_token(mut self, token: Option<String>) -> Self {
        self.supplied_shutdown_token = token;
        self
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

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Conflict,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Forbidden,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            kind: MaintenanceRouteErrorKind::Internal,
            message: logs::redact_sensitive(&error.to_string()),
        }
    }
}

pub async fn begin_update_drain(
    state: &Arc<DaemonState>,
    reason: String,
    owner: String,
) -> Result<DaemonTurnActivitySummary, BeginUpdateDrainError> {
    if state
        .core
        .update_drain
        .acquire(reason, owner)
        .await
        .is_none()
    {
        return Err(BeginUpdateDrainError::AlreadyActive);
    }

    let activity = match daemon_turn_activity_summary(state).await {
        Ok(activity) => activity,
        Err(error) => {
            let _ = state.core.update_drain.release().await;
            return Err(BeginUpdateDrainError::ActivityUnavailable(error));
        }
    };
    if !activity.idle {
        let _ = state.core.update_drain.release().await;
        return Err(BeginUpdateDrainError::Busy);
    }
    Ok(activity)
}

pub async fn release_update_drain(state: &DaemonState) -> bool {
    state.core.update_drain.release().await
}

pub async fn reject_new_execution_during_maintenance(state: &DaemonState) -> Result<(), Error> {
    state.core.update_drain.reject_if_draining().await
}

pub async fn post_message_update_drain_reason(state: &DaemonState) -> Option<String> {
    state
        .core
        .update_drain
        .snapshot()
        .await
        .map(|drain| drain.reason)
}

pub async fn acquire_linux_sandbox_prepare_drain(
    state: &Arc<DaemonState>,
) -> Result<MaintenanceDrainPermit, MaintenanceDrainError> {
    if state
        .core
        .update_drain
        .acquire("linux_sandbox_runtime_prepare", "execution_api")
        .await
        .is_none()
    {
        return Err(MaintenanceDrainError::AlreadyActive);
    }
    let permit = MaintenanceDrainPermit {
        state: Arc::clone(state),
        released: false,
    };

    let activity = match daemon_sandbox_work_activity_summary(state).await {
        Ok(activity) => activity,
        Err(error) => {
            let _ = permit.release().await;
            return Err(MaintenanceDrainError::ActivityUnavailable(error));
        }
    };
    if sandbox_work_is_active(&activity) {
        let _ = permit.release().await;
        return Err(MaintenanceDrainError::SandboxWorkActive);
    }

    Ok(permit)
}

pub async fn request_daemon_shutdown(
    state: Arc<DaemonState>,
    reason: String,
) -> Result<DaemonTurnActivitySummary, DaemonShutdownError> {
    let acquired_drain = state
        .core
        .update_drain
        .acquire(&reason, "daemon_shutdown")
        .await
        .is_some();

    for session_id in state.running_session_ids().await {
        if let Some(tx) = state.session_scheduler_sender(session_id).await {
            let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
            let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;
        }
    }

    for _ in 0..10 {
        let activity = match daemon_turn_activity_summary(&state).await {
            Ok(activity) => activity,
            Err(error) => {
                release_shutdown_drain_on_error(&state, acquired_drain).await;
                return Err(DaemonShutdownError::ActivityUnavailable(error));
            }
        };
        if activity.running_turn_count == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    if let Err(error) = reconcile_running_turns_with_reason(&state, &reason).await {
        if acquired_drain {
            let _ = state.core.update_drain.release().await;
        }
        return Err(DaemonShutdownError::Reconcile(error));
    }
    let activity = match daemon_turn_activity_summary(&state).await {
        Ok(activity) => activity,
        Err(error) => {
            release_shutdown_drain_on_error(&state, acquired_drain).await;
            return Err(DaemonShutdownError::ActivityUnavailable(error));
        }
    };

    spawn_deferred_daemon_shutdown(state, reason, Duration::from_millis(100));
    Ok(activity)
}

fn sandbox_work_is_active(activity: &DaemonSandboxWorkActivitySummary) -> bool {
    activity.active
}

async fn release_shutdown_drain_on_error(state: &DaemonState, acquired_drain: bool) {
    if acquired_drain {
        let _ = state.core.update_drain.release().await;
    }
}

impl ExecutionHandle {
    pub async fn begin_update_drain_for_route(
        &self,
        req: BeginUpdateDrainRouteRequest,
    ) -> Result<BeginUpdateDrainRouteResult, MaintenanceRouteError> {
        if !req.confirm {
            return Err(MaintenanceRouteError::bad_request("confirm required"));
        }
        let reason = req
            .reason
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "daemon_update".to_string());
        let owner = req
            .owner
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let activity = begin_update_drain(&self.state, reason, owner)
            .await
            .map_err(begin_update_drain_route_error)?;
        Ok(BeginUpdateDrainRouteResult {
            acquired: true,
            activity,
        })
    }

    pub async fn release_update_drain_for_route(
        &self,
        req: ReleaseUpdateDrainRouteRequest,
    ) -> Result<ReleaseUpdateDrainRouteResult, MaintenanceRouteError> {
        if !req.confirm {
            return Err(MaintenanceRouteError::bad_request("confirm required"));
        }
        Ok(ReleaseUpdateDrainRouteResult {
            released: release_update_drain(self.state.as_ref()).await,
        })
    }

    pub async fn request_daemon_shutdown_for_route(
        &self,
        req: ShutdownDaemonRouteRequest,
    ) -> Result<ShutdownDaemonRouteResult, MaintenanceRouteError> {
        if !req.confirm {
            return Err(MaintenanceRouteError::bad_request("confirm required"));
        }
        if !self.local_shutdown_token_authorized(req.supplied_shutdown_token.as_deref()) {
            return Err(MaintenanceRouteError::forbidden(
                "local desktop shutdown token required",
            ));
        }
        let reason = req
            .reason
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "desktop_quit".to_string());
        let activity = request_daemon_shutdown(std::sync::Arc::clone(&self.state), reason)
            .await
            .map_err(daemon_shutdown_route_error)?;
        Ok(ShutdownDaemonRouteResult {
            accepted: true,
            activity,
        })
    }

    fn local_shutdown_token_authorized(&self, supplied: Option<&str>) -> bool {
        let Some(expected) = self.state.core.local_shutdown_token.as_deref() else {
            return false;
        };
        supplied.is_some_and(|value| value == expected)
    }

    pub async fn begin_update_drain(
        &self,
        reason: String,
        owner: String,
    ) -> Result<DaemonTurnActivitySummary, BeginUpdateDrainError> {
        begin_update_drain(&self.state, reason, owner).await
    }

    pub async fn release_update_drain(&self) -> bool {
        release_update_drain(self.state.as_ref()).await
    }

    pub async fn reject_new_execution_during_maintenance(&self) -> Result<(), Error> {
        reject_new_execution_during_maintenance(self.state.as_ref()).await
    }

    pub async fn acquire_linux_sandbox_prepare_drain(
        &self,
    ) -> Result<MaintenanceDrainPermit, MaintenanceDrainError> {
        acquire_linux_sandbox_prepare_drain(&self.state).await
    }

    pub async fn daemon_turn_activity_summary(&self) -> Result<DaemonTurnActivitySummary, Error> {
        daemon_turn_activity_summary(&self.state).await
    }

    pub async fn request_daemon_shutdown(
        &self,
        reason: String,
    ) -> Result<DaemonTurnActivitySummary, DaemonShutdownError> {
        request_daemon_shutdown(std::sync::Arc::clone(&self.state), reason).await
    }
}

fn begin_update_drain_route_error(error: BeginUpdateDrainError) -> MaintenanceRouteError {
    match error {
        BeginUpdateDrainError::AlreadyActive => {
            MaintenanceRouteError::conflict("daemon update drain already active")
        }
        BeginUpdateDrainError::ActivityUnavailable(error) => MaintenanceRouteError::internal(error),
        BeginUpdateDrainError::Busy => MaintenanceRouteError::conflict(
            "daemon has queued or running turns; update drain was not acquired",
        ),
    }
}

fn daemon_shutdown_route_error(error: DaemonShutdownError) -> MaintenanceRouteError {
    match error {
        DaemonShutdownError::ActivityUnavailable(error) | DaemonShutdownError::Reconcile(error) => {
            MaintenanceRouteError::internal(error)
        }
    }
}

#[cfg(test)]
mod tests;
