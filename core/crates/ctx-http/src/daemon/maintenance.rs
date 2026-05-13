use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;

use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::{
    daemon_sandbox_work_activity_summary, daemon_turn_activity_summary,
    reconcile_running_turns_with_reason, spawn_deferred_daemon_shutdown, AppState,
    DaemonSandboxWorkActivitySummary, DaemonTurnActivitySummary,
};

pub(crate) struct MaintenanceDrainPermit {
    state: Arc<AppState>,
}

impl MaintenanceDrainPermit {
    pub(crate) async fn release(self) -> bool {
        self.state.core.update_drain.release().await
    }
}

#[derive(Debug)]
pub(crate) enum BeginUpdateDrainError {
    AlreadyActive,
    ActivityUnavailable(Error),
    Busy,
}

#[derive(Debug)]
pub(crate) enum MaintenanceDrainError {
    AlreadyActive,
    ActivityUnavailable(Error),
    SandboxWorkActive,
}

#[derive(Debug)]
pub(crate) enum DaemonShutdownError {
    ActivityUnavailable(Error),
    Reconcile(Error),
}

pub(crate) async fn begin_update_drain(
    state: &Arc<AppState>,
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

pub(crate) async fn release_update_drain(state: &AppState) -> bool {
    state.core.update_drain.release().await
}

pub(crate) async fn reject_new_execution_during_maintenance(state: &AppState) -> Result<(), Error> {
    state.core.update_drain.reject_if_draining().await
}

pub(crate) async fn post_message_update_drain_reason(state: &AppState) -> Option<String> {
    state
        .core
        .update_drain
        .snapshot()
        .await
        .map(|drain| drain.reason)
}

pub(crate) async fn acquire_linux_sandbox_prepare_drain(
    state: &Arc<AppState>,
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

pub(crate) async fn request_daemon_shutdown(
    state: Arc<AppState>,
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

async fn release_shutdown_drain_on_error(state: &AppState, acquired_drain: bool) {
    if acquired_drain {
        let _ = state.core.update_drain.release().await;
    }
}

#[cfg(test)]
mod tests;
