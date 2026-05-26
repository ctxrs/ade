use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use anyhow::Context;
use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, Session, SessionEvent, SessionHeadDelta, SessionSummary,
    SessionSummaryDelta, SessionTurn, SessionTurnToolSummary, SubagentInvocation, Task,
    TaskDeltaKind, Workspace, Worktree, WorktreeVcsSnapshot,
};
use ctx_observability::ops_events::OpsEvents;
use ctx_observability::perf_telemetry::PerfTelemetry;
use ctx_provider_runtime::{
    provider_install_tracker::ProviderInstallOpsEvent, ProviderRuntime, ProviderRuntimeHost,
};
use ctx_session_runtime::runtime::{
    SessionEventPublicationHost, SessionReplayCursor, SessionRuntime, SessionTaskDeltaRefreshHost,
};
use ctx_session_vcs_service::vcs::SessionVcsDiffBaseQuery;
use ctx_store::{Store, StoreManager};
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;
use ctx_worktree_vcs_service::{
    GitStatusSnapshot, WorktreeDiffBaseResolution, WorktreeVcsDiffSummaryCounts,
};
use tokio::sync::{mpsc, Mutex};

use super::{
    scheduler::SessionSchedulerWorkerHost,
    state::{
        session_store_access_anyhow, ProtectedWorkspaceStoreLookup, SessionStoreLookup,
        WorktreeFileCompletionsCache,
    },
};

pub(in crate::daemon) type SessionArtifactsFuture<T> =
    Pin<Box<dyn Future<Output = T> + Send + 'static>>;
#[derive(Clone)]
pub struct SessionFileCompletionsHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    worktree_file_completions_cache: WorktreeFileCompletionsCache,
    perf_telemetry: PerfTelemetry,
    data_root: PathBuf,
    daemon_url: String,
    harness: Arc<HarnessRuntimeManager>,
}

pub(in crate::daemon) struct SessionFileCompletionsHandleParts {
    pub(in crate::daemon) global_store: Store,
    pub(in crate::daemon) session_stores: SessionStoreLookup,
    pub(in crate::daemon) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(in crate::daemon) worktree_file_completions_cache: WorktreeFileCompletionsCache,
    pub(in crate::daemon) perf_telemetry: PerfTelemetry,
    pub(in crate::daemon) data_root: PathBuf,
    pub(in crate::daemon) daemon_url: String,
    pub(in crate::daemon) harness: Arc<HarnessRuntimeManager>,
}

impl SessionFileCompletionsHandle {
    pub(in crate::daemon) fn new(parts: SessionFileCompletionsHandleParts) -> Self {
        Self {
            global_store: parts.global_store,
            session_stores: parts.session_stores,
            workspace_stores: parts.workspace_stores,
            worktree_file_completions_cache: parts.worktree_file_completions_cache,
            perf_telemetry: parts.perf_telemetry,
            data_root: parts.data_root,
            daemon_url: parts.daemon_url,
            harness: parts.harness,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores.existing_session_store(session_id).await
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) fn worktree_file_completions_cache(
        &self,
    ) -> &WorktreeFileCompletionsCache {
        &self.worktree_file_completions_cache
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }
}

pub(in crate::daemon) struct SessionTitleModelModeHandleParts {
    pub(in crate::daemon) global_store: Store,
    pub(in crate::daemon) session_stores: SessionStoreLookup,
    pub(in crate::daemon) workspace_stores: ProtectedWorkspaceStoreLookup,
    pub(in crate::daemon) session_runtime:
        Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    pub(in crate::daemon) active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    pub(in crate::daemon) provider_runtime: Arc<ProviderRuntime>,
    pub(in crate::daemon) ops_events: OpsEvents,
    pub(in crate::daemon) data_root: PathBuf,
    pub(in crate::daemon) daemon_url: String,
    pub(in crate::daemon) auth_token: Option<String>,
    pub(in crate::daemon) harness: Arc<HarnessRuntimeManager>,
}

#[derive(Clone)]
pub struct SessionTitleModelModeHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    provider_runtime: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    harness: Arc<HarnessRuntimeManager>,
}

impl SessionTitleModelModeHandle {
    pub(in crate::daemon) fn new(parts: SessionTitleModelModeHandleParts) -> Self {
        Self {
            global_store: parts.global_store,
            session_stores: parts.session_stores,
            workspace_stores: parts.workspace_stores,
            session_runtime: parts.session_runtime,
            active_snapshot: parts.active_snapshot,
            provider_runtime: parts.provider_runtime,
            ops_events: parts.ops_events,
            data_root: parts.data_root,
            daemon_url: parts.daemon_url,
            auth_token: parts.auth_token,
            harness: parts.harness,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn active_snapshot(&self) -> &WorkspaceActiveSnapshotHub {
        self.active_snapshot.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.provider_runtime.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn auth_token(&self) -> Option<&String> {
        self.auth_token.as_ref()
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }

    pub(in crate::daemon) async fn get_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<Workspace>> {
        self.global_store().get_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn session_store_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.session_stores.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(crate::daemon::SessionStoreAccessError::LookupUnavailable(error)) => Err(error),
            Err(crate::daemon::SessionStoreAccessError::StoreUnavailable) => {
                anyhow::bail!("workspace store unavailable")
            }
        }
    }

    pub(in crate::daemon) async fn session_store_for_write_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.existing_session_store_for_write(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(crate::daemon::SessionStoreAccessError::LookupUnavailable(error)) => Err(error),
            Err(crate::daemon::SessionStoreAccessError::StoreUnavailable) => {
                anyhow::bail!("workspace store unavailable")
            }
        }
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn store_for_task(&self, task_id: TaskId) -> anyhow::Result<Store> {
        let workspace_id = self
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await?
            .with_context(|| format!("workspace missing for task {}", task_id.0))?;
        self.store_for_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_session(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Store> {
        self.session_store_or_none(session_id)
            .await?
            .with_context(|| format!("workspace missing for session {}", session_id.0))
    }

    pub(in crate::daemon) async fn remember_session_meta(&self, session: &Session) {
        self.session_runtime.remember_session_meta(session).await;
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        let host = SessionTitleModelModePublicationHost::new(self.clone());
        self.session_runtime
            .publish_event_with_host(&host, event)
            .await;
    }

    pub(in crate::daemon) async fn emit_workspace_task_upsert(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        let mut task: Option<Task> = None;
        let store = self.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            self.emit_workspace_archived_task_upsert(task).await?;
        }
        Ok(())
    }

    async fn emit_workspace_archived_task_upsert(&self, task: &Task) -> anyhow::Result<()> {
        let store = self.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary)
            .await;
        Ok(())
    }

    pub(in crate::daemon) async fn load_provider_model_catalog_for_execution_environment(
        &self,
        workspace: &Workspace,
        provider_id: &str,
        execution_environment: ExecutionEnvironment,
    ) -> Result<Option<ctx_session_tools::model_resolution::ModelCatalog>, String> {
        crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
            self,
            workspace,
            provider_id,
            execution_environment,
        )
        .await
    }
}

#[derive(Clone)]
pub(in crate::daemon) struct SessionMessageSchedulerSpawner {
    host: Weak<SessionSchedulerWorkerHost>,
}

impl SessionMessageSchedulerSpawner {
    pub(in crate::daemon) fn new(host: Weak<SessionSchedulerWorkerHost>) -> Self {
        Self { host }
    }

    pub(in crate::daemon) async fn ensure_scheduler(
        &self,
        runtime: &SessionRuntime<crate::daemon::scheduler::SchedulerCommand>,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        let host = self.host.clone();
        runtime
            .ensure_scheduler(session, move |session, rx| {
                crate::daemon::scheduler::session_worker(host, session, rx)
            })
            .await
    }
}

#[derive(Clone)]
pub struct SessionMessageCommandHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    update_drain: Arc<UpdateDrainCoordinator>,
    data_root: PathBuf,
    title_model_mode: SessionTitleModelModeHandle,
    scheduler_spawner: SessionMessageSchedulerSpawner,
}

impl SessionMessageCommandHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        session_stores: SessionStoreLookup,
        session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
        update_drain: Arc<UpdateDrainCoordinator>,
        data_root: PathBuf,
        title_model_mode: SessionTitleModelModeHandle,
        scheduler_spawner: SessionMessageSchedulerSpawner,
    ) -> Self {
        Self {
            global_store,
            session_stores,
            session_runtime,
            update_drain,
            data_root,
            title_model_mode,
            scheduler_spawner,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn remember_session_meta(&self, session: &Session) {
        self.session_runtime.remember_session_meta(session).await;
    }

    pub(in crate::daemon) async fn session_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<ctx_session_tools::order_seq::OrderSeqState>> {
        self.session_runtime
            .get_order_seq_state(store, session_id)
            .await
    }

    pub(in crate::daemon) async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.session_runtime.is_running(session_id).await
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        self.title_model_mode.publish_event(event).await;
    }

    pub(in crate::daemon) async fn post_message_update_drain_reason(&self) -> Option<String> {
        self.update_drain.snapshot().await.map(|drain| drain.reason)
    }

    pub(in crate::daemon) async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>> {
        self.session_runtime.scheduler_sender(session_id).await
    }

    pub(in crate::daemon) async fn ensure_scheduler(
        &self,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        self.scheduler_spawner
            .ensure_scheduler(&self.session_runtime, session)
            .await
    }

    #[cfg(test)]
    pub(in crate::daemon) async fn ensure_scheduler_for_test<F, Fut>(
        &self,
        session: Session,
        spawn_worker: F,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>
    where
        F: FnOnce(Session, mpsc::Receiver<crate::daemon::scheduler::SchedulerCommand>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.session_runtime
            .ensure_scheduler(session, spawn_worker)
            .await
    }

    #[cfg(test)]
    pub(in crate::daemon) async fn subscribe_session_event_head_for_test(
        &self,
        session_id: SessionId,
    ) -> tokio::sync::watch::Receiver<i64> {
        self.session_runtime
            .subscribe_session_event_head(session_id)
            .await
    }

    pub(in crate::daemon) async fn maybe_schedule_first_message_title_generation(
        &self,
        store: &Store,
        session: Session,
        message: &Message,
    ) {
        if let Ok(count) = store.count_user_messages_for_session(session.id).await {
            if count == 1 {
                let _ = self
                    .title_model_mode
                    .schedule_session_title_generation(session, message.content.clone(), false)
                    .await;
            }
        }
    }
}

impl ProviderRuntimeHost for SessionTitleModelModeHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        crate::daemon::provider_capability_hosts::current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            self.ops_events(),
            events,
        );
    }
}

struct SessionTitleModelModePublicationHost {
    handle: SessionTitleModelModeHandle,
    task_delta_refresh_host: Arc<SessionTitleModelModeTaskDeltaRefreshHost>,
}

impl SessionTitleModelModePublicationHost {
    fn new(handle: SessionTitleModelModeHandle) -> Self {
        Self {
            task_delta_refresh_host: Arc::new(SessionTitleModelModeTaskDeltaRefreshHost {
                handle: handle.clone(),
            }),
            handle,
        }
    }
}

#[async_trait::async_trait]
impl SessionEventPublicationHost for SessionTitleModelModePublicationHost {
    type TaskDeltaRefreshHost = SessionTitleModelModeTaskDeltaRefreshHost;

    fn task_delta_refresh_host(&self) -> Arc<Self::TaskDeltaRefreshHost> {
        Arc::clone(&self.task_delta_refresh_host)
    }

    async fn load_session(&self, session_id: SessionId) -> Option<Session> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store.get_session(session_id).await.ok().flatten()
    }

    async fn list_turn_tool_summaries_for_turn(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Vec<SessionTurnToolSummary> {
        let Ok(store) = self.handle.store_for_session(session_id).await else {
            return Vec::new();
        };
        store
            .list_turn_tool_summaries_for_turns(session_id, std::slice::from_ref(&turn_id))
            .await
            .unwrap_or_default()
    }

    async fn cached_turn_for_read(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Option<SessionTurn> {
        self.handle
            .active_snapshot()
            .get_cached_session_head_for_read(session_id)
            .await
            .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
    }

    async fn load_turn(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Option<SessionTurn> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store
            .get_session_turn(session_id, turn_id)
            .await
            .ok()
            .flatten()
    }

    async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        let cursor = self
            .handle
            .active_snapshot()
            .session_replay_cursor(workspace_id, session_id)
            .await;
        SessionReplayCursor {
            last_event_seq: cursor.last_event_seq,
            projection_rev: cursor.projection_rev,
        }
    }

    async fn load_projection_rev(&self, session_id: SessionId) -> Option<i64> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store.get_session_projection_rev(session_id).await.ok()
    }

    async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        delta: SessionHeadDelta,
        durable: bool,
    ) {
        self.handle
            .active_snapshot()
            .publish_session_head_delta(workspace_id, session, delta, durable)
            .await;
    }

    async fn publish_session_summary_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionSummaryDelta,
    ) {
        self.handle
            .active_snapshot()
            .publish_session_summary_delta(workspace_id, delta)
            .await;
    }
}

pub(in crate::daemon) struct SessionTitleModelModeTaskDeltaRefreshHost {
    handle: SessionTitleModelModeHandle,
}

#[async_trait::async_trait]
impl SessionTaskDeltaRefreshHost for SessionTitleModelModeTaskDeltaRefreshHost {
    async fn emit_task_delta_refresh(&self, task_id: TaskId) {
        let store = match self.handle.store_for_task(task_id).await {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh store lookup failed: {err:?}"
                );
                return;
            }
        };
        match store.get_workspace_active_task_summary(task_id).await {
            Ok(Some(summary)) => {
                let _ = self
                    .handle
                    .active_snapshot()
                    .publish_task_delta(
                        summary.task.workspace_id,
                        summary.task,
                        TaskDeltaKind::Updated,
                    )
                    .await;
            }
            Ok(None) => match store.get_task(task_id).await {
                Ok(Some(task)) => {
                    let kind = if task.archived_at.is_some() {
                        TaskDeltaKind::Archived
                    } else {
                        TaskDeltaKind::Updated
                    };
                    let _ = self
                        .handle
                        .active_snapshot()
                        .publish_task_delta(task.workspace_id, task, kind)
                        .await;
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
        }
    }
}

#[derive(Clone)]
pub struct SessionSubagentReadHandle {
    session_stores: SessionStoreLookup,
}

impl SessionSubagentReadHandle {
    pub(in crate::daemon) fn new(session_stores: SessionStoreLookup) -> Self {
        Self { session_stores }
    }

    async fn load_session_store_and_parent(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<(Store, Session)>> {
        let store = match self.session_stores.existing_session_store(session_id).await {
            Ok(store) => store,
            Err(crate::daemon::SessionStoreAccessError::NotFound) => return Ok(None),
            Err(error) => return Err(session_store_access_anyhow(error)),
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        Ok(Some((store, session)))
    }

    pub(in crate::daemon) async fn list_session_subagents_for_request(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Vec<SessionSummary>>> {
        let Some((store, session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        store.list_subagent_sessions(session.id).await.map(Some)
    }

    pub(in crate::daemon) async fn list_session_subagent_invocations_for_request(
        &self,
        session_id: SessionId,
        turn_id: Option<TurnId>,
    ) -> anyhow::Result<Option<Vec<SubagentInvocation>>> {
        let Some((store, session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        store
            .list_subagent_invocations_for_session(session.id, turn_id)
            .await
            .map(Some)
    }

    pub(in crate::daemon) async fn get_session_subagent_invocation_for_request(
        &self,
        session_id: SessionId,
        invocation_id: &str,
    ) -> anyhow::Result<Option<SubagentInvocation>> {
        let Some((store, _session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        let Some(invocation) = store.get_subagent_invocation(invocation_id).await? else {
            return Ok(None);
        };
        if invocation.parent_session_id != session_id {
            return Ok(None);
        }
        Ok(Some(invocation))
    }
}

pub(in crate::daemon) type SessionSubagentMcpReadFuture<T> =
    Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub(in crate::daemon) type SessionSubagentMcpReadProviderTimeout =
    Arc<dyn Fn() -> SessionSubagentMcpReadFuture<Duration> + Send + Sync>;
pub(in crate::daemon) type SessionSubagentMcpReadLegacyContextWindowRejectCounter =
    Arc<dyn Fn(String) -> SessionSubagentMcpReadFuture<()> + Send + Sync>;

#[derive(Clone)]
pub struct SessionSubagentMcpReadHandle {
    session_stores: SessionStoreLookup,
    provider_inactivity_timeout: SessionSubagentMcpReadProviderTimeout,
    emit_legacy_context_window_key_reject: SessionSubagentMcpReadLegacyContextWindowRejectCounter,
}

impl SessionSubagentMcpReadHandle {
    pub(in crate::daemon) fn new(
        session_stores: SessionStoreLookup,
        provider_inactivity_timeout: SessionSubagentMcpReadProviderTimeout,
        emit_legacy_context_window_key_reject: SessionSubagentMcpReadLegacyContextWindowRejectCounter,
    ) -> Self {
        Self {
            session_stores,
            provider_inactivity_timeout,
            emit_legacy_context_window_key_reject,
        }
    }

    async fn load_parent_session(
        &self,
        parent_id: SessionId,
    ) -> Result<(Store, Session), crate::daemon::sessions::subagents::SubagentError> {
        let store = match self.session_stores.existing_session_store(parent_id).await {
            Ok(store) => store,
            Err(crate::daemon::SessionStoreAccessError::NotFound) => {
                return Err(crate::daemon::sessions::subagents::not_found(
                    "parent session not found",
                ));
            }
            Err(error) => {
                return Err(crate::daemon::sessions::subagents::internal_api_error(
                    session_store_access_anyhow(error),
                ));
            }
        };
        let parent = store
            .get_session(parent_id)
            .await
            .map_err(crate::daemon::sessions::subagents::internal_api_error)?
            .ok_or_else(|| {
                crate::daemon::sessions::subagents::not_found("parent session not found")
            })?;
        Ok((store, parent))
    }

    async fn provider_inactivity_timeout(&self) -> Duration {
        (self.provider_inactivity_timeout)().await
    }

    pub(in crate::daemon) async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::ScopedMcpSessionAccessError> {
        self.session_stores
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await
    }

    pub(in crate::daemon) async fn list_agents(
        &self,
        parent_id: SessionId,
    ) -> Result<
        Vec<crate::daemon::sessions::subagents::AgentSummary>,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let subs = store
            .list_subagent_sessions(parent.id)
            .await
            .map_err(crate::daemon::sessions::subagents::internal_api_error)?;
        let mut agents = Vec::with_capacity(subs.len());
        for sub in subs {
            let (summary, _latest_turn) = crate::daemon::sessions::subagents::build_agent_summary(
                &store,
                sub.id,
                &sub.title,
                inactivity_timeout,
            )
            .await?;
            agents.push(summary);
        }
        Ok(agents)
    }

    pub(in crate::daemon) async fn get_agent(
        &self,
        parent_id: SessionId,
        req: crate::daemon::sessions::subagents::GetAgentReq,
    ) -> Result<
        crate::daemon::sessions::subagents::GetAgentResp,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let child = crate::daemon::sessions::subagents::resolve_child_agent_session(
            &store,
            &parent,
            &req.agent_id,
        )
        .await?;
        let detail = crate::daemon::sessions::subagents::build_agent_detail_for_mcp_read(
            &store,
            &parent,
            &child,
            inactivity_timeout,
            &self.emit_legacy_context_window_key_reject,
        )
        .await?;
        Ok(crate::daemon::sessions::subagents::GetAgentResp { agent: detail })
    }

    pub(in crate::daemon) async fn wait_agent(
        &self,
        parent_id: SessionId,
        req: crate::daemon::sessions::subagents::WaitAgentReq,
    ) -> Result<
        crate::daemon::sessions::subagents::WaitAgentResp,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let agent_ids = ctx_subagent_service::normalize_wait_agent_ids(
            req.agent_id.as_deref(),
            req.agent_ids.as_deref(),
        )
        .map_err(|error| {
            crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                error,
            )
        })?;
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let targets =
            crate::daemon::sessions::subagents::collect_wait_targets(&store, &parent, &agent_ids)
                .await?;
        let mode = ctx_subagent_service::parse_wait_mode(req.mode.as_deref()).map_err(|error| {
            crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                error,
            )
        })?;
        let until =
            ctx_subagent_service::parse_wait_until(req.until.as_deref()).map_err(|error| {
                crate::daemon::sessions::subagents::api_error(
                    crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                    error,
                )
            })?;
        if req.since_seq.is_some() && targets.len() != 1 {
            return Err(crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                "since_seq is only supported with a single agent_id",
            ));
        }

        let timeout_ms = req.timeout_ms.unwrap_or(30_000);
        let mut details = self
            .collect_wait_details(&store, &parent, &targets, inactivity_timeout)
            .await?;
        let thresholds = subagent_wait_update_thresholds(&details, until, req.since_seq);

        if ctx_subagent_service::wait_predicate_satisfied(
            &subagent_agent_wait_details(&details),
            mode,
            until,
            &thresholds,
        ) {
            return Ok(subagent_wait_response("matched", mode, until, details));
        }
        if timeout_ms == 0 {
            return Ok(subagent_wait_response("timeout", mode, until, details));
        }

        let started_at = Instant::now();
        while started_at.elapsed() < Duration::from_millis(timeout_ms) {
            tokio::time::sleep(Duration::from_millis(100)).await;
            details = self
                .collect_wait_details(&store, &parent, &targets, inactivity_timeout)
                .await?;
            if ctx_subagent_service::wait_predicate_satisfied(
                &subagent_agent_wait_details(&details),
                mode,
                until,
                &thresholds,
            ) {
                return Ok(subagent_wait_response("matched", mode, until, details));
            }
        }

        Ok(subagent_wait_response("timeout", mode, until, details))
    }

    async fn collect_wait_details(
        &self,
        store: &Store,
        parent: &Session,
        targets: &[Session],
        inactivity_timeout: Duration,
    ) -> Result<
        Vec<crate::daemon::sessions::subagents::AgentDetail>,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let mut details = Vec::with_capacity(targets.len());
        for target in targets {
            details.push(
                crate::daemon::sessions::subagents::build_agent_detail_for_mcp_read(
                    store,
                    parent,
                    target,
                    inactivity_timeout,
                    &self.emit_legacy_context_window_key_reject,
                )
                .await?,
            );
        }
        Ok(details)
    }
}

fn subagent_wait_update_thresholds(
    details: &[crate::daemon::sessions::subagents::AgentDetail],
    until: ctx_subagent_service::AgentWaitUntil,
    since_seq: Option<i64>,
) -> HashMap<String, i64> {
    let mut thresholds = HashMap::new();
    match until {
        ctx_subagent_service::AgentWaitUntil::Terminal => {}
        ctx_subagent_service::AgentWaitUntil::Update => {
            if let Some(since_seq) = since_seq {
                thresholds.insert(details[0].agent.agent_id.clone(), since_seq);
            } else {
                for detail in details {
                    thresholds.insert(detail.agent.agent_id.clone(), detail.agent.last_event_seq);
                }
            }
        }
    }
    thresholds
}

fn subagent_wait_response(
    wait_status: &str,
    mode: ctx_subagent_service::AgentWaitMode,
    until: ctx_subagent_service::AgentWaitUntil,
    results: Vec<crate::daemon::sessions::subagents::AgentDetail>,
) -> crate::daemon::sessions::subagents::WaitAgentResp {
    crate::daemon::sessions::subagents::WaitAgentResp {
        wait_status: wait_status.to_string(),
        mode: mode.as_str().to_string(),
        until: until.as_str().to_string(),
        results,
    }
}

fn subagent_agent_wait_details(
    details: &[crate::daemon::sessions::subagents::AgentDetail],
) -> Vec<ctx_subagent_service::AgentWaitDetail<'_>> {
    details
        .iter()
        .map(|detail| ctx_subagent_service::AgentWaitDetail {
            agent_id: &detail.agent.agent_id,
            has_current_run: detail.agent.current_run_id.is_some(),
            has_latest_result: detail.agent.latest_result_status.is_some(),
            last_event_seq: detail.agent.last_event_seq,
        })
        .collect()
}

#[derive(Clone)]
pub struct SessionReadModelsHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    stores: StoreManager,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    tool_output_spool_dir: PathBuf,
    perf_telemetry: PerfTelemetry,
}

impl SessionReadModelsHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        session_stores: SessionStoreLookup,
        stores: StoreManager,
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        tool_output_spool_dir: PathBuf,
        perf_telemetry: PerfTelemetry,
    ) -> Self {
        Self {
            global_store,
            session_stores,
            stores,
            active_snapshot,
            tool_output_spool_dir,
            perf_telemetry,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn session_stores(&self) -> &SessionStoreLookup {
        &self.session_stores
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn active_snapshot(&self) -> &WorkspaceActiveSnapshotHub {
        self.active_snapshot.as_ref()
    }

    pub(in crate::daemon) fn tool_output_spool_dir(&self) -> &Path {
        &self.tool_output_spool_dir
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }
}

#[derive(Clone)]
pub struct SessionArtifactsHandle {
    lookup: SessionStoreLookup,
    tool_output_spool_dir: PathBuf,
    effects: Arc<SessionArtifactEffects>,
}

impl SessionArtifactsHandle {
    pub(in crate::daemon) fn new(
        lookup: SessionStoreLookup,
        tool_output_spool_dir: PathBuf,
        effects: Arc<SessionArtifactEffects>,
    ) -> Self {
        Self {
            lookup,
            tool_output_spool_dir,
            effects,
        }
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.lookup.existing_session_store(session_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.lookup
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::ScopedMcpSessionAccessError> {
        self.lookup
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await
    }

    pub(in crate::daemon) fn session_tool_output_spool_dir(
        &self,
        session_id: SessionId,
    ) -> PathBuf {
        self.tool_output_spool_dir.join(session_id.0.to_string())
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        self.effects.publish_event(event).await;
    }
}

pub(in crate::daemon) struct SessionArtifactEffects {
    publish_event: Arc<dyn Fn(SessionEvent) -> SessionArtifactsFuture<()> + Send + Sync>,
}

impl SessionArtifactEffects {
    pub(in crate::daemon) fn new(
        publish_event: Arc<dyn Fn(SessionEvent) -> SessionArtifactsFuture<()> + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self { publish_event })
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        (self.publish_event)(event).await;
    }
}

pub(in crate::daemon) type SessionVcsFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub(in crate::daemon) type SessionVcsWorktreeBoolEffect =
    Arc<dyn Fn(Worktree) -> SessionVcsFuture<anyhow::Result<bool>> + Send + Sync>;
pub(in crate::daemon) type SessionVcsGitStatusEffect = Arc<
    dyn Fn(Worktree, bool, bool) -> SessionVcsFuture<anyhow::Result<GitStatusSnapshot>>
        + Send
        + Sync,
>;
pub(in crate::daemon) type SessionVcsCommitEffect =
    Arc<dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<String>> + Send + Sync>;
pub(in crate::daemon) type SessionVcsDiffEffect =
    Arc<dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<String>> + Send + Sync>;
pub(in crate::daemon) type SessionVcsDiffSummaryEffect = Arc<
    dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<WorktreeVcsDiffSummaryCounts>>
        + Send
        + Sync,
>;
pub(in crate::daemon) type SessionVcsDiffBaseEffect = Arc<
    dyn Fn(Worktree, SessionVcsDiffBaseQuery) -> SessionVcsFuture<WorktreeDiffBaseResolution>
        + Send
        + Sync,
>;
pub(in crate::daemon) type SessionVcsPatchEffect =
    Arc<dyn Fn(Worktree, String, bool) -> SessionVcsFuture<anyhow::Result<()>> + Send + Sync>;
pub(in crate::daemon) type SessionVcsSnapshotEffect =
    Arc<dyn Fn(WorktreeId) -> SessionVcsFuture<Option<WorktreeVcsSnapshot>> + Send + Sync>;
pub(in crate::daemon) type SessionVcsCompatMetricEffect =
    Arc<dyn Fn(&'static str, &'static str) -> SessionVcsFuture<()> + Send + Sync>;
pub(in crate::daemon) type SessionVcsNoRepoClassifier =
    Arc<dyn Fn(&anyhow::Error) -> bool + Send + Sync>;

pub(in crate::daemon) struct SessionVcsEffectsParts {
    pub(in crate::daemon) worktree_has_vcs_repo: SessionVcsWorktreeBoolEffect,
    pub(in crate::daemon) load_git_status_snapshot: SessionVcsGitStatusEffect,
    pub(in crate::daemon) resolve_worktree_commit: SessionVcsCommitEffect,
    pub(in crate::daemon) diff_worktree_for_session: SessionVcsDiffEffect,
    pub(in crate::daemon) diff_worktree_summary_for_session: SessionVcsDiffSummaryEffect,
    pub(in crate::daemon) resolve_worktree_diff_base: SessionVcsDiffBaseEffect,
    pub(in crate::daemon) apply_worktree_vcs_session_patch: SessionVcsPatchEffect,
    pub(in crate::daemon) cached_worktree_vcs_snapshot: SessionVcsSnapshotEffect,
    pub(in crate::daemon) emit_compat_payload_reject_counter: SessionVcsCompatMetricEffect,
    pub(in crate::daemon) is_no_vcs_repo_error: SessionVcsNoRepoClassifier,
}

pub(in crate::daemon) struct SessionVcsEffects {
    worktree_has_vcs_repo: SessionVcsWorktreeBoolEffect,
    load_git_status_snapshot: SessionVcsGitStatusEffect,
    resolve_worktree_commit: SessionVcsCommitEffect,
    diff_worktree_for_session: SessionVcsDiffEffect,
    diff_worktree_summary_for_session: SessionVcsDiffSummaryEffect,
    resolve_worktree_diff_base: SessionVcsDiffBaseEffect,
    apply_worktree_vcs_session_patch: SessionVcsPatchEffect,
    cached_worktree_vcs_snapshot: SessionVcsSnapshotEffect,
    emit_compat_payload_reject_counter: SessionVcsCompatMetricEffect,
    is_no_vcs_repo_error: SessionVcsNoRepoClassifier,
}

impl SessionVcsEffects {
    pub(in crate::daemon) fn new(parts: SessionVcsEffectsParts) -> Arc<Self> {
        Arc::new(Self {
            worktree_has_vcs_repo: parts.worktree_has_vcs_repo,
            load_git_status_snapshot: parts.load_git_status_snapshot,
            resolve_worktree_commit: parts.resolve_worktree_commit,
            diff_worktree_for_session: parts.diff_worktree_for_session,
            diff_worktree_summary_for_session: parts.diff_worktree_summary_for_session,
            resolve_worktree_diff_base: parts.resolve_worktree_diff_base,
            apply_worktree_vcs_session_patch: parts.apply_worktree_vcs_session_patch,
            cached_worktree_vcs_snapshot: parts.cached_worktree_vcs_snapshot,
            emit_compat_payload_reject_counter: parts.emit_compat_payload_reject_counter,
            is_no_vcs_repo_error: parts.is_no_vcs_repo_error,
        })
    }
}

#[derive(Clone)]
pub struct SessionVcsHandle {
    lookup: SessionStoreLookup,
    effects: Arc<SessionVcsEffects>,
}

impl SessionVcsHandle {
    pub(in crate::daemon) fn new(
        lookup: SessionStoreLookup,
        effects: Arc<SessionVcsEffects>,
    ) -> Self {
        Self { lookup, effects }
    }

    pub(in crate::daemon) async fn session_store_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.lookup.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    pub(in crate::daemon) async fn session_store_for_write_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self
            .lookup
            .existing_session_store_for_write(session_id)
            .await
        {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    pub(in crate::daemon) async fn worktree_has_vcs_repo(
        &self,
        worktree: &Worktree,
    ) -> anyhow::Result<bool> {
        (self.effects.worktree_has_vcs_repo)(worktree.clone()).await
    }

    pub(in crate::daemon) async fn load_git_status_snapshot(
        &self,
        worktree: &Worktree,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> anyhow::Result<GitStatusSnapshot> {
        (self.effects.load_git_status_snapshot)(
            worktree.clone(),
            include_untracked_files,
            include_entries,
        )
        .await
    }

    pub(in crate::daemon) async fn resolve_worktree_commit(
        &self,
        worktree: &Worktree,
        revision: &str,
    ) -> anyhow::Result<String> {
        (self.effects.resolve_worktree_commit)(worktree.clone(), revision.to_string()).await
    }

    pub(in crate::daemon) async fn diff_worktree_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<String> {
        (self.effects.diff_worktree_for_session)(worktree.clone(), base_commit_sha.to_string())
            .await
    }

    pub(in crate::daemon) async fn diff_worktree_summary_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
        (self.effects.diff_worktree_summary_for_session)(
            worktree.clone(),
            base_commit_sha.to_string(),
        )
        .await
    }

    pub(in crate::daemon) async fn resolve_worktree_diff_base(
        &self,
        worktree: &Worktree,
        query: SessionVcsDiffBaseQuery,
    ) -> WorktreeDiffBaseResolution {
        (self.effects.resolve_worktree_diff_base)(worktree.clone(), query).await
    }

    pub(in crate::daemon) async fn apply_worktree_vcs_session_patch(
        &self,
        worktree: &Worktree,
        patch: &str,
        reverse_patch: bool,
    ) -> anyhow::Result<()> {
        (self.effects.apply_worktree_vcs_session_patch)(
            worktree.clone(),
            patch.to_string(),
            reverse_patch,
        )
        .await
    }

    pub(in crate::daemon) async fn cached_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        (self.effects.cached_worktree_vcs_snapshot)(worktree_id).await
    }

    pub(in crate::daemon) async fn emit_compat_payload_reject_counter(
        &self,
        surface: &'static str,
        issue: &'static str,
    ) {
        (self.effects.emit_compat_payload_reject_counter)(surface, issue).await;
    }

    pub(in crate::daemon) fn is_no_vcs_repo_error(&self, error: &anyhow::Error) -> bool {
        (self.effects.is_no_vcs_repo_error)(error)
    }
}
