use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{Session, SessionEvent};
use ctx_session_runtime::runtime::SessionRuntimeCacheDebugStats;
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::Store;

use crate::daemon::state::{DaemonState, ProtectedWorkspaceStoreLookup, SessionStoreLookup};
use crate::daemon::ProviderWorkspaceLaunchRuntime;

impl DaemonState {
    pub(in crate::daemon) fn session_scheduler_worker_host(
        self: &Arc<Self>,
    ) -> Arc<crate::daemon::scheduler::SessionSchedulerWorkerHost> {
        Arc::clone(self.scheduler_worker_host.get_or_init(|| {
            let workspace_stores = ProtectedWorkspaceStoreLookup::new(
                self.core.stores.clone(),
                Arc::clone(&self.sessions),
                Arc::clone(&self.transport.merge_queue),
            );
            let session_stores =
                SessionStoreLookup::new(self.global_store().clone(), workspace_stores.clone());
            let provider_launch_runtime = Arc::new(ProviderWorkspaceLaunchRuntime::new(
                self.core.data_root.clone(),
                self.core.daemon_url.clone(),
                self.core.auth_token.clone(),
                workspace_stores.clone(),
                Arc::clone(&self.providers),
                self.telemetry.ops_events.clone(),
                Arc::clone(&self.execution.harness),
            ));
            Arc::new(crate::daemon::scheduler::SessionSchedulerWorkerHost::new(
                crate::daemon::scheduler::SessionSchedulerWorkerHostParts {
                    session_stores,
                    session_runtime: Arc::clone(&self.sessions),
                    workspace_stores,
                    active_snapshot: Arc::clone(&self.workspaces.workspace_active_snapshot),
                    global_store: self.global_store().clone(),
                    providers: Arc::clone(&self.providers),
                    provider_launch_runtime,
                    worktree_bootstrap_gates: Arc::clone(&self.workspaces.worktree_bootstrap_gates),
                    storage_guard: Arc::clone(&self.core.storage_guard),
                    update_drain: Arc::clone(&self.core.update_drain),
                    mcp_auth: Arc::clone(&self.core.mcp_auth),
                    perf_telemetry: self.telemetry.perf_telemetry.clone(),
                    telemetry: self.telemetry.telemetry.clone(),
                    provider_unknown_events: self.telemetry.provider_unknown_events.clone(),
                    resource_sampler: Arc::clone(&self.telemetry.resource_sampler),
                    tool_output_spool_enabled: self.core.tool_output_spool_enabled,
                    tool_output_spool_dir: self.core.tool_output_spool_dir.clone(),
                    ops_events: self.telemetry.ops_events.clone(),
                },
            ))
        }))
    }

    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        super::runtime::publish_event(self, event).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        super::runtime::refresh_session_head_cache(self, session_id).await;
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<Mutex<()>> {
        self.sessions.task_session_creation_lock(task_id).await
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.sessions.remember_session_meta(session).await;
    }

    pub async fn session_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<OrderSeqState>> {
        self.sessions.get_order_seq_state(store, session_id).await
    }

    pub async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.sessions.is_running(session_id).await
    }

    pub async fn running_session_ids(&self) -> Vec<SessionId> {
        self.sessions.list_running_sessions().await
    }

    pub async fn session_scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>> {
        self.sessions.scheduler_sender(session_id).await
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        self.sessions.subscribe_session_event_head(session_id).await
    }

    pub async fn provider_inactivity_timeout(&self) -> Duration {
        self.sessions.provider_inactivity_timeout().await
    }

    pub async fn session_cache_debug_stats(&self) -> SessionRuntimeCacheDebugStats {
        self.sessions.cache_debug_stats().await
    }

    pub async fn cached_session_ids_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<SessionId> {
        self.sessions
            .cached_session_ids_for_workspace(workspace_id)
            .await
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        let host_weak = Arc::downgrade(&self.session_scheduler_worker_host());
        self.sessions
            .ensure_scheduler(session, move |session, rx| {
                crate::daemon::scheduler::session_worker(host_weak, session, rx)
            })
            .await
    }
}
