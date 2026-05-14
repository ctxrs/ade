use std::sync::{Arc, Weak};

use ctx_core::ids::TaskId;
use ctx_core::models::TaskDeltaKind;
use ctx_session_service::runtime::SessionTaskDeltaRefreshHost;

use crate::daemon::state::DaemonState;

pub(super) struct HttpTaskDeltaRefreshHost {
    state: Weak<DaemonState>,
}

impl HttpTaskDeltaRefreshHost {
    pub(super) fn new(state: &Arc<DaemonState>) -> Self {
        Self {
            state: Arc::downgrade(state),
        }
    }
}

#[async_trait::async_trait]
impl SessionTaskDeltaRefreshHost for HttpTaskDeltaRefreshHost {
    async fn emit_task_delta_refresh(&self, task_id: TaskId) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        match state.store_for_task(task_id).await {
            Ok(store) => match store.get_workspace_active_task_summary(task_id).await {
                Ok(Some(summary)) => {
                    let _ = state
                        .emit_workspace_task_delta(summary.task, TaskDeltaKind::Updated)
                        .await;
                }
                Ok(None) => match store.get_task(task_id).await {
                    Ok(Some(task)) => {
                        let kind = if task.archived_at.is_some() {
                            TaskDeltaKind::Archived
                        } else {
                            TaskDeltaKind::Updated
                        };
                        let _ = state.emit_workspace_task_delta(task, kind).await;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            task_id = %task_id.0,
                            "workspace task delta refresh read failed: {err:?}"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        "workspace task delta refresh summary read failed: {err:?}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh store lookup failed: {err:?}"
                );
            }
        }
    }
}
