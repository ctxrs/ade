use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify, Semaphore};
use tokio::task::JoinHandle;

use crate::git_status::GitStatusSnapshot;
use crate::ops_events::{OpsEvent, OpsEvents};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::provider_guard;
use crate::provider_restart;
use crate::provider_usage;
use crate::resource_governance::ResourceGovernanceRuntime;
use crate::runtime_adapters::{
    CtxExecutionHarness, CtxRuntimeEventSink, CtxRuntimeMetricsSink, DefaultWarmupOperations,
};
use crate::scheduler::SchedulerCommand;
use crate::telemetry::Telemetry;
use crate::terminals::TerminalManager;
use crate::web_sessions::WebSessionManager;
use ctx_core::ids::{SessionId, TaskId, WorkspaceAttachmentId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Session, SessionEvent, SessionHeadSnapshot, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorktreeVcsSnapshot, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::{
    InstallErrorCode, InstallEventLevel, InstallId, InstallProgressEvent, InstallState,
    InstallStateKind, InstallTarget,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use ctx_providers::ask_user_question::AskUserQuestionBroker;
use ctx_resource_utilization::ResourceSampler;
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;

mod builder;
mod installs;
mod metrics;
mod types;

pub(crate) use types::{
    provider_inactivity_timeout_from_env, worktree_vcs_enabled_from_env,
    worktree_vcs_scheduler_concurrency_from_env, ActiveTaskRefreshEntry,
    AttachmentMaterializationTask, WorktreeBootstrapGate, WorktreeVcsDirtyBits,
};

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
pub use types::{
    AppRuntimeFlags, AppState, CacheSweepConfig, CacheSweepStats, CachedFileCompletions,
    CachedProviderOptions, CachedProviderVerify, CoreState, ExecutionRuntime,
    GitStatusSnapshotCacheEntry, ProviderRuntime, SessionHeadCacheKey, SessionPinState,
    SessionRuntime, StoreLookup, TelemetryRuntime, TimedEntry, TransportRuntime, UpdateDrainState,
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry, WorkspaceRuntime,
    WorktreeVcsSchedulerRuntime, WorktreeVcsSnapshotCacheEntry,
};

impl AppState {
    pub fn global_store(&self) -> &Store {
        self.core.stores.global()
    }

    pub async fn update_drain_snapshot(&self) -> Option<UpdateDrainState> {
        self.core.update_drain.lock().await.clone()
    }

    pub async fn acquire_update_drain(
        &self,
        reason: impl Into<String>,
        owner: impl Into<String>,
    ) -> Option<UpdateDrainState> {
        let mut guard = self.core.update_drain.lock().await;
        if guard.is_some() {
            return None;
        }
        let state = UpdateDrainState {
            reason: reason.into(),
            owner: owner.into(),
            acquired_at_ms: current_time_ms(),
        };
        *guard = Some(state.clone());
        Some(state)
    }

    pub async fn release_update_drain(&self) -> bool {
        self.core.update_drain.lock().await.take().is_some()
    }

    pub async fn reject_if_update_draining(&self) -> Result<()> {
        if let Some(drain) = self.update_drain_snapshot().await {
            anyhow::bail!(
                "daemon maintenance is in progress; retry after it completes (reason={}, owner={})",
                drain.reason,
                drain.owner
            );
        }
        Ok(())
    }

    async fn protected_workspace_store_ids(&self) -> HashSet<WorkspaceId> {
        let mut active_sessions: HashSet<SessionId> = HashSet::new();
        {
            let set = self.sessions.running_sessions.lock().await;
            active_sessions.extend(set.iter().copied());
        }
        {
            let map = self.sessions.schedulers.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.broadcasters.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.sessions.session_event_heads.lock().await;
            active_sessions.extend(map.keys().copied());
        }

        let mut active_workspaces: HashSet<WorkspaceId> = HashSet::new();
        let mut missing = Vec::new();
        {
            let cache = self.sessions.session_meta_cache.lock().await;
            for session_id in &active_sessions {
                if let Some(entry) = cache.get(session_id) {
                    active_workspaces.insert(entry.value.workspace_id);
                } else {
                    missing.push(*session_id);
                }
            }
        }
        for session_id in missing {
            if let Ok(Some(workspace_id)) = self
                .global_store()
                .get_workspace_id_for_session(session_id)
                .await
            {
                active_workspaces.insert(workspace_id);
            }
        }
        active_workspaces.extend(self.transport.merge_queue.running_workspaces().await);

        active_workspaces
    }

    pub async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> Result<Store> {
        match self.lookup_workspace_store(workspace_id).await {
            StoreLookup::Found(store) => Ok(store),
            StoreLookup::Missing | StoreLookup::Deleting => {
                anyhow::bail!("workspace {} not found", workspace_id.0)
            }
            StoreLookup::Unavailable(err) => Err(err),
        }
    }

    pub async fn lookup_workspace_store(&self, workspace_id: WorkspaceId) -> StoreLookup {
        match self
            .core
            .stores
            .workspace_access_outcome(workspace_id)
            .await
        {
            Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Access(access)) => {
                if access.kind.triggers_open_side_effects() {
                    let mut protected_workspaces = self.protected_workspace_store_ids().await;
                    protected_workspaces.insert(workspace_id);
                    self.core
                        .stores
                        .evict_workspaces_to_cap(&protected_workspaces)
                        .await;
                }
                StoreLookup::Found(access.store)
            }
            Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Missing) => StoreLookup::Missing,
            Ok(ctx_store::manager::WorkspaceStoreAccessOutcome::Deleting) => StoreLookup::Deleting,
            Err(err) => StoreLookup::Unavailable(err),
        }
    }

    pub async fn lookup_session_store(&self, session_id: SessionId) -> StoreLookup {
        let workspace_id = match self
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
        {
            Ok(Some(workspace_id)) => workspace_id,
            Ok(None) => return StoreLookup::Missing,
            Err(err) => return StoreLookup::Unavailable(err),
        };
        match self.lookup_workspace_store(workspace_id).await {
            StoreLookup::Found(store) => StoreLookup::Found(store),
            StoreLookup::Missing | StoreLookup::Deleting => StoreLookup::Deleting,
            StoreLookup::Unavailable(err) => StoreLookup::Unavailable(err),
        }
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> Result<Store> {
        let workspace_id = self
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await?
            .with_context(|| format!("workspace missing for task {}", task_id.0))?;
        self.store_for_workspace(workspace_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> Result<Store> {
        match self.lookup_session_store(session_id).await {
            StoreLookup::Found(store) => Ok(store),
            StoreLookup::Missing | StoreLookup::Deleting => {
                anyhow::bail!("workspace missing for session {}", session_id.0)
            }
            StoreLookup::Unavailable(err) => Err(err),
        }
    }

    pub async fn store_for_worktree(&self, worktree_id: WorktreeId) -> Result<Store> {
        let workspace_id = self
            .global_store()
            .get_workspace_id_for_worktree(worktree_id)
            .await?
            .with_context(|| format!("workspace missing for worktree {}", worktree_id.0))?;
        self.store_for_workspace(workspace_id).await
    }

    pub async fn sweep_idle_caches(
        &self,
        now: Instant,
        config: CacheSweepConfig,
    ) -> CacheSweepStats {
        let mut stats = CacheSweepStats::default();
        let running_sessions = {
            let set = self.sessions.running_sessions.lock().await;
            set.iter().copied().collect::<HashSet<_>>()
        };
        {
            let mut cache = self.sessions.session_head_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_head_evicted += expired.len();
        }
        {
            let mut cache = self.sessions.session_meta_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_meta_evicted += expired.len();
        }
        {
            let mut map = self.sessions.schedulers.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if entry.value.is_closed()
                        || now.duration_since(entry.last_access) >= config.session_ttl
                    {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.schedulers_evicted += expired.len();
        }
        {
            let mut map = self.sessions.broadcasters.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.broadcasters_evicted += expired.len();
        }
        {
            let mut map = self.sessions.session_event_heads.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.session_event_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.file_completions_cache.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.file_completions_evicted += expired.len();
        }
        {
            let mut cache = self
                .workspaces
                .workspace_file_completions_cache
                .lock()
                .await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_file_completions_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.git_status_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.git_status_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_vcs_snapshots.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_vcs_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_snapshot_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_snapshot_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.workspace_active_heads_cache.lock().await;
            let expired: Vec<WorkspaceId> = cache
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.workspace_ttl {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                cache.remove(workspace_id);
            }
            stats.workspace_heads_evicted += expired.len();
        }
        {
            let mut cache = self.workspaces.worktree_bootstrap_gates.lock().await;
            let expired: Vec<WorktreeId> = cache
                .iter()
                .filter_map(|(worktree_id, entry)| {
                    if now.duration_since(entry.last_access) >= config.session_ttl {
                        Some(*worktree_id)
                    } else {
                        None
                    }
                })
                .collect();
            for worktree_id in &expired {
                cache.remove(worktree_id);
            }
            stats.worktree_bootstrap_evicted += expired.len();
        }
        let active_workspaces = self.protected_workspace_store_ids().await;

        stats.workspace_stores_evicted = self
            .core
            .stores
            .evict_idle_workspaces(config.workspace_ttl, &active_workspaces)
            .await;

        self.emit_cache_evicted("session_head", stats.session_head_evicted)
            .await;
        self.emit_cache_evicted("session_meta", stats.session_meta_evicted)
            .await;
        self.emit_cache_evicted("scheduler", stats.schedulers_evicted)
            .await;
        self.emit_cache_evicted("broadcaster", stats.broadcasters_evicted)
            .await;
        self.emit_cache_evicted("session_event_head", stats.session_event_heads_evicted)
            .await;
        self.emit_cache_evicted("file_completions", stats.file_completions_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_file_completions",
            stats.workspace_file_completions_evicted,
        )
        .await;
        self.emit_cache_evicted("git_status", stats.git_status_evicted)
            .await;
        self.emit_cache_evicted(
            "workspace_active_snapshot",
            stats.workspace_snapshot_evicted,
        )
        .await;
        self.emit_cache_evicted("workspace_active_heads", stats.workspace_heads_evicted)
            .await;
        self.emit_cache_evicted("worktree_bootstrap", stats.worktree_bootstrap_evicted)
            .await;
        self.emit_cache_evicted("workspace_store", stats.workspace_stores_evicted)
            .await;

        stats
    }
}

#[cfg(test)]
mod tests;
