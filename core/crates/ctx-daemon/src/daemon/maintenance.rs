use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;

use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary,
    reconcile_running_turns_with_reason, spawn_deferred_daemon_shutdown,
    DaemonSandboxWorkActivitySummary, DaemonState, DaemonTurnActivitySummary, ExecutionHandle,
};

pub struct MaintenanceDrainPermit {
    state: Arc<DaemonState>,
}

impl MaintenanceDrainPermit {
    pub async fn release(self) -> bool {
        self.state.core.update_drain.release().await
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

    let activity = match daemon_sandbox_work_activity_summary(state).await {
        Ok(activity) => activity,
        Err(error) => {
            let _ = state.core.update_drain.release().await;
            return Err(MaintenanceDrainError::ActivityUnavailable(error));
        }
    };
    if sandbox_work_is_active(&activity) {
        let _ = state.core.update_drain.release().await;
        return Err(MaintenanceDrainError::SandboxWorkActive);
    }

    Ok(MaintenanceDrainPermit {
        state: Arc::clone(state),
    })
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

#[cfg(test)]
mod tests;
