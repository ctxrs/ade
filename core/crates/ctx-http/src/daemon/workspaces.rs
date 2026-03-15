use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    SessionHeadSnapshot, Task, TaskDeltaKind, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot,
    WorkspaceActiveTaskSummary, Worktree, WorktreeVcsSnapshot,
};
use ctx_lsp::Language as LspLanguage;
use ctx_store::Store;

use crate::git_status;
use crate::workspace_active_snapshot::WorkspaceActiveSnapshotHub;

use super::state::{
    AppState, TimedEntry, WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorkspaceRuntime, WorktreeBootstrapGate,
};

struct WorkspaceSnapshotHydrationPayload {
    snapshot_rev: i64,
    archived_rev: i64,
    tasks: Vec<WorkspaceActiveTaskSummary>,
    heads: Vec<SessionHeadSnapshot>,
}

#[async_trait]
trait WorkspaceSnapshotHydrationStore {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)>;
    async fn list_active_page(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)>;
    async fn list_active_session_ids(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<ctx_core::ids::SessionId>>;
    async fn get_active_head(
        &self,
        session_id: ctx_core::ids::SessionId,
    ) -> Result<Option<SessionHeadSnapshot>>;
}

#[async_trait]
impl WorkspaceSnapshotHydrationStore for Store {
    async fn get_snapshot_state(&self, workspace_id: WorkspaceId) -> Result<(i64, i64)> {
        self.get_workspace_active_snapshot_state(workspace_id).await
    }

    async fn list_active_page(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
        self.list_workspace_active_page(workspace_id, limit).await
    }

    async fn list_active_session_ids(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<ctx_core::ids::SessionId>> {
        self.list_workspace_active_session_ids(workspace_id).await
    }

    async fn get_active_head(
        &self,
        session_id: ctx_core::ids::SessionId,
    ) -> Result<Option<SessionHeadSnapshot>> {
        self.get_active_snapshot_head(session_id).await
    }
}

async fn load_workspace_snapshot_hydration_payload<S: WorkspaceSnapshotHydrationStore + Sync>(
    store: &S,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceSnapshotHydrationPayload> {
    let (snapshot_rev, archived_rev) = store.get_snapshot_state(workspace_id).await?;
    let (tasks, _) = store.list_active_page(workspace_id, i64::MAX).await?;
    let session_ids = store.list_active_session_ids(workspace_id).await?;
    let mut heads = Vec::new();
    for session_id in session_ids {
        match store.get_active_head(session_id).await {
            Ok(Some(head)) => heads.push(head),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "ctx_http.workspace_active_snapshot",
                    workspace_id = %workspace_id.0,
                    session_id = %session_id.0,
                    error = ?err,
                    "skipping active head during workspace hydration",
                );
            }
        }
    }
    Ok(WorkspaceSnapshotHydrationPayload {
        snapshot_rev,
        archived_rev,
        tasks,
        heads,
    })
}

async fn apply_workspace_snapshot_hydration_payload(
    hub: &WorkspaceActiveSnapshotHub,
    workspace_id: WorkspaceId,
    payload: WorkspaceSnapshotHydrationPayload,
) {
    hub.hydrate_snapshot(
        workspace_id,
        payload.snapshot_rev,
        payload.archived_rev,
        payload.tasks,
        payload.heads,
    )
    .await;
}

impl WorkspaceRuntime {
    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            (
                entry.value.snapshot.snapshot_rev,
                entry.value.snapshot.archived_rev,
            )
        })
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.snapshot.clone()
        })
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        let workspace_id = snapshot.workspace_id;
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveSnapshotCacheEntry { snapshot }),
        );
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.batch.clone()
        })
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        let workspace_id = batch.workspace_id;
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveHeadCacheEntry { batch }),
        );
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        let mut cache = self.worktree_vcs_snapshots.lock().await;
        cache.get_mut(&worktree_id).map(|entry| {
            entry.touch();
            entry.value.snapshot.clone()
        })
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        if previous == next {
            return;
        }
        let mut evicted = Vec::new();
        {
            let mut active = self.worktree_vcs_active.lock().await;
            for worktree_id in previous.difference(next) {
                if let Some(count) = active.get_mut(worktree_id) {
                    if *count <= 1 {
                        active.remove(worktree_id);
                        evicted.push(*worktree_id);
                    } else {
                        *count -= 1;
                    }
                }
            }
            for worktree_id in next.difference(previous) {
                let entry = active.entry(*worktree_id).or_insert(0);
                *entry += 1;
            }
        }
        if !evicted.is_empty() {
            {
                let mut cache = self.worktree_vcs_snapshots.lock().await;
                for worktree_id in &evicted {
                    cache.remove(worktree_id);
                }
            }
            {
                let mut gens = self.worktree_vcs_summary_gen.lock().await;
                for worktree_id in &evicted {
                    gens.remove(worktree_id);
                }
            }
            self.workspace_active_snapshot
                .drop_worktree_vcs_snapshots(&evicted)
                .await;
        }
    }

    pub async fn is_worktree_vcs_active(&self, worktree_id: WorktreeId) -> bool {
        let active = self.worktree_vcs_active.lock().await;
        active.get(&worktree_id).copied().unwrap_or(0) > 0
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
    ) {
        if !self
            .workspace_active_snapshot
            .needs_hydration(workspace_id)
            .await
        {
            return;
        }
        let store = match state.store_for_workspace(workspace_id).await {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot (store lookup)"
                );
                return;
            }
        };
        let payload = match load_workspace_snapshot_hydration_payload(&store, workspace_id).await {
            Ok(payload) => payload,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    "failed to load workspace snapshot hydration payload: {err:#}"
                );
                return;
            }
        };
        apply_workspace_snapshot_hydration_payload(
            self.workspace_active_snapshot.as_ref(),
            workspace_id,
            payload,
        )
        .await;
    }

    pub async fn register_worktree_bootstrap(
        &self,
        worktree_id: WorktreeId,
        wait_for_completion: bool,
    ) {
        let (done_tx, _) = watch::channel(false);
        let mut map = self.worktree_bootstrap_gates.lock().await;
        map.insert(
            worktree_id,
            TimedEntry::new(WorktreeBootstrapGate {
                wait_for_completion,
                done_tx,
            }),
        );
    }

    pub async fn finish_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let gate = {
            let mut map = self.worktree_bootstrap_gates.lock().await;
            map.remove(&worktree_id)
        };
        if let Some(gate) = gate {
            let _ = gate.value.done_tx.send(true);
        }
    }

    pub async fn wait_for_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        let mut done_rx = {
            let mut map = self.worktree_bootstrap_gates.lock().await;
            let Some(gate) = map.get_mut(&worktree_id) else {
                return;
            };
            gate.touch();
            if !gate.value.wait_for_completion {
                return;
            }
            gate.value.done_tx.subscribe()
        };
        if *done_rx.borrow() {
            return;
        }
        let _ = done_rx.changed().await;
    }

    pub async fn cleanup_workspace(&self, state: &AppState, workspace_id: WorkspaceId) {
        let session_ids = {
            let cache = state.sessions.session_meta_cache.lock().await;
            cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if entry.value.workspace_id == workspace_id {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for session_id in session_ids {
            state.cleanup_session(session_id).await;
        }
        {
            let mut cache = self.workspace_active_snapshot_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_active_heads_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_file_completions_cache.lock().await;
            cache.remove(&workspace_id);
        }
        self.workspace_active_snapshot
            .remove_workspace(workspace_id)
            .await;
    }

    pub async fn emit_workspace_task_upsert(
        &self,
        state: &AppState,
        task_id: TaskId,
    ) -> Result<()> {
        let mut task: Option<Task> = None;
        let store = state.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.workspace_active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.workspace_active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            let _ = self.emit_workspace_archived_task_upsert(state, task).await;
        }
        Ok(())
    }

    pub async fn emit_workspace_task_delete(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        if let Err(err) = state.store_for_workspace(workspace_id).await {
            tracing::warn!(
                workspace_id = %workspace_id.0,
                task_id = %task_id.0,
                "workspace task delete store missing: {err:#}"
            );
            return;
        }
        self.workspace_active_snapshot
            .publish_active_task_delete(workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        state: &AppState,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        let updated = match state.store_for_workspace(workspace_id).await {
            Ok(store) => match store
                .bump_workspace_archived_snapshot_rev(workspace_id)
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(
                        workspace_id = %workspace_id.0,
                        task_id = %task_id.0,
                        "workspace archived delete read model update failed: {err:#}"
                    );
                    false
                }
            },
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    task_id = %task_id.0,
                    "workspace archived delete read model store missing: {err:#}"
                );
                false
            }
        };
        if updated {
            self.workspace_active_snapshot
                .publish_archived_task_delete(workspace_id, task_id)
                .await;
        }
    }

    async fn emit_workspace_archived_task_upsert(
        &self,
        state: &AppState,
        task: &Task,
    ) -> Result<()> {
        let store = state.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.workspace_active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary)
            .await;
        Ok(())
    }

    pub async fn ensure_git_status_watcher(&self, state: &Arc<AppState>, worktree: Worktree) {
        let mut watchers = self.git_status_watchers.lock().await;
        if !watchers.insert(worktree.id) {
            return;
        }
        let worktree_id = worktree.id;
        let state = Arc::clone(state);
        tokio::spawn(async move {
            if let Err(err) = git_status::run_git_status_watcher(state.clone(), worktree).await {
                tracing::warn!(worktree_id = %worktree_id.0, "git status watcher failed: {err:#}");
            }
            state
                .workspaces
                .release_git_status_watcher(worktree_id)
                .await;
        });
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        let mut watchers = self.git_status_watchers.lock().await;
        watchers.remove(&worktree_id);
    }
}

impl AppState {
    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        self.workspaces
            .cached_workspace_active_snapshot_state(workspace_id)
            .await
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        self.workspaces
            .cached_workspace_active_snapshot(workspace_id)
            .await
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        self.workspaces
            .cache_workspace_active_snapshot(snapshot)
            .await;
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        self.workspaces
            .cached_workspace_active_heads(workspace_id)
            .await
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        self.workspaces.cache_workspace_active_heads(batch).await;
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        self.workspaces.get_worktree_vcs_snapshot(worktree_id).await
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        self.workspaces
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn is_worktree_vcs_active(&self, worktree_id: WorktreeId) -> bool {
        self.workspaces.is_worktree_vcs_active(worktree_id).await
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(&self, workspace_id: WorkspaceId) {
        self.workspaces
            .ensure_workspace_active_snapshot_hydrated(self, workspace_id)
            .await;
    }

    pub async fn register_worktree_bootstrap(
        &self,
        worktree_id: WorktreeId,
        wait_for_completion: bool,
    ) {
        self.workspaces
            .register_worktree_bootstrap(worktree_id, wait_for_completion)
            .await;
    }

    pub async fn finish_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        self.workspaces.finish_worktree_bootstrap(worktree_id).await;
    }

    pub async fn wait_for_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        self.workspaces
            .wait_for_worktree_bootstrap(worktree_id)
            .await;
    }

    pub async fn cleanup_workspace(&self, workspace_id: WorkspaceId) {
        // Best-effort: workspace deletion should attempt to clean up its harness container + volume,
        // but must not fail deletion if podman is unavailable.
        let _ = self.execution.harness.stop_container(workspace_id).await;
        let _ = self
            .execution
            .harness
            .remove_workspace_volume(workspace_id)
            .await;
        self.workspaces.cleanup_workspace(self, workspace_id).await;
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> Result<()> {
        self.workspaces
            .emit_workspace_task_upsert(self, task_id)
            .await
    }

    pub async fn emit_workspace_task_delta(&self, task: Task, kind: TaskDeltaKind) -> bool {
        self.workspaces
            .workspace_active_snapshot
            .publish_task_delta(task.workspace_id, task, kind)
            .await
    }

    pub async fn emit_workspace_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        self.workspaces
            .emit_workspace_task_delete(self, workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        self.workspaces
            .emit_workspace_archived_task_delete(self, workspace_id, task_id)
            .await;
    }

    pub async fn ensure_git_status_watcher(self: &Arc<Self>, worktree: Worktree) {
        self.workspaces
            .ensure_git_status_watcher(self, worktree)
            .await;
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        self.workspaces
            .release_git_status_watcher(worktree_id)
            .await;
    }

    pub async fn ensure_lsp_diagnostics_forwarder(
        self: &Arc<Self>,
        root: PathBuf,
        lang: LspLanguage,
    ) {
        if !self.core.lsp.enabled() {
            return;
        }
        let key = format!("{}:{}", root.to_string_lossy(), lang.id());
        {
            let mut set = self.transport.lsp_diag_forwarders.lock().await;
            if set.contains(&key) {
                return;
            }
            set.insert(key);
        }

        let state = self.clone();
        tokio::spawn(async move {
            let mut rx = match state
                .core
                .lsp
                .subscribe_diagnostics_for_language(&root, lang)
                .await
            {
                Ok(v) => v,
                Err(_) => return,
            };
            loop {
                let update = match rx.recv().await {
                    Ok(u) => u,
                    Err(_) => break,
                };
                let url = match url::Url::parse(&update.uri.to_string()) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                if url.scheme() != "file" {
                    continue;
                }
                let Ok(abs_path) = url.to_file_path() else {
                    continue;
                };

                let watchers = state.core.buffers.watchers_for_abs_path(&abs_path).await;
                if watchers.is_empty() {
                    continue;
                }

                let diagnostics_json = match serde_json::to_value(&update.diagnostics) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for (sid, rel) in watchers {
                    let msg = serde_json::json!({
                        "type": "lsp_diagnostics",
                        "session_id": sid.0.to_string(),
                        "path": rel.to_string_lossy(),
                        "diagnostics": diagnostics_json,
                    });
                    let _ = state.transport.lsp_diag_broadcaster.send(msg);
                }
            }
        });
    }
}

#[cfg(test)]
mod hydration_tests {
    use super::{
        apply_workspace_snapshot_hydration_payload, load_workspace_snapshot_hydration_payload,
        WorkspaceSnapshotHydrationPayload, WorkspaceSnapshotHydrationStore,
    };
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use chrono::Utc;
    use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SessionActivityState, SessionHeadSnapshot, SessionHeadWindow,
        SessionMetadata, SessionSnapshotSummary, SessionStatus, SessionTurnStatus, Task,
        TaskStatus, WorkspaceActiveTaskSummary,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::workspace_active_snapshot::WorkspaceActiveSnapshotHub;

    struct FakeHydrationStore {
        snapshot_state: (i64, i64),
        tasks: Vec<WorkspaceActiveTaskSummary>,
        session_ids: Vec<SessionId>,
        heads_by_session: HashMap<SessionId, SessionHeadSnapshot>,
        failing_heads: HashMap<SessionId, &'static str>,
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl WorkspaceSnapshotHydrationStore for FakeHydrationStore {
        async fn get_snapshot_state(&self, _workspace_id: WorkspaceId) -> Result<(i64, i64)> {
            self.calls.lock().unwrap().push("snapshot_state");
            Ok(self.snapshot_state)
        }

        async fn list_active_page(
            &self,
            _workspace_id: WorkspaceId,
            limit: i64,
        ) -> Result<(Vec<WorkspaceActiveTaskSummary>, i64)> {
            self.calls.lock().unwrap().push("active_page");
            assert_eq!(limit, i64::MAX);
            Ok((self.tasks.clone(), self.tasks.len() as i64))
        }

        async fn list_active_session_ids(
            &self,
            _workspace_id: WorkspaceId,
        ) -> Result<Vec<SessionId>> {
            self.calls.lock().unwrap().push("active_session_ids");
            Ok(self.session_ids.clone())
        }

        async fn get_active_head(
            &self,
            session_id: SessionId,
        ) -> Result<Option<SessionHeadSnapshot>> {
            self.calls.lock().unwrap().push("active_head");
            if let Some(message) = self.failing_heads.get(&session_id) {
                return Err(anyhow!(*message));
            }
            Ok(self.heads_by_session.get(&session_id).cloned())
        }
    }

    fn test_task(workspace_id: WorkspaceId, task_id: TaskId, session_id: SessionId) -> Task {
        let now = Utc::now();
        Task {
            id: task_id,
            workspace_id,
            title: "hydrate".to_string(),
            description: None,
            status: TaskStatus::Pending,
            exec_plan_id: None,
            primary_session_id: Some(session_id),
            primary_worktree_id: Some(WorktreeId::new()),
            created_at: now,
            updated_at: now,
            archived_at: None,
            assistant_seen_at: None,
            last_activity_at: Some(now),
            last_assistant_message_at: None,
            has_active_session: true,
        }
    }

    fn test_session_metadata(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> SessionMetadata {
        let now = Utc::now();
        SessionMetadata {
            id: session_id,
            task_id,
            workspace_id,
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: String::new(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_summary(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> WorkspaceActiveTaskSummary {
        WorkspaceActiveTaskSummary {
            task: test_task(workspace_id, task_id, session_id),
            primary_session: SessionSnapshotSummary {
                session: test_session_metadata(workspace_id, task_id, session_id),
                last_message_at: None,
                last_message_preview: Some("canonical-summary".to_string()),
                last_event_seq: Some(44),
                state_rev: 44,
                activity: SessionActivityState {
                    is_working: false,
                    last_turn_status: Some(SessionTurnStatus::Completed),
                },
                unread: None,
            },
            primary_session_head: None,
            sessions: Vec::new(),
            sort_at: Utc::now(),
        }
    }

    fn test_head(
        workspace_id: WorkspaceId,
        task_id: TaskId,
        session_id: SessionId,
    ) -> SessionHeadSnapshot {
        SessionHeadSnapshot {
            session: test_session_metadata(workspace_id, task_id, session_id),
            turns: Vec::new(),
            tool_summaries: Vec::new(),
            events: Vec::new(),
            messages: Vec::new(),
            last_event_seq: 44,
            state_rev: 44,
            activity: SessionActivityState {
                is_working: false,
                last_turn_status: Some(SessionTurnStatus::Completed),
            },
            has_more_turns: false,
            history_cursor: None,
            has_more_history: false,
            summary_checkpoint: None,
            head_window: SessionHeadWindow::default(),
        }
    }

    #[tokio::test]
    async fn workspace_hydration_payload_uses_canonical_page_and_preserves_snapshot_rev() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let summary = test_summary(workspace_id, task_id, session_id);
        let head = test_head(workspace_id, task_id, session_id);
        let store = FakeHydrationStore {
            snapshot_state: (17, 4),
            tasks: vec![summary],
            session_ids: vec![session_id],
            heads_by_session: HashMap::from([(session_id, head.clone())]),
            failing_heads: HashMap::new(),
            calls: Mutex::new(Vec::new()),
        };

        let payload = load_workspace_snapshot_hydration_payload(&store, workspace_id)
            .await
            .expect("expected hydration payload");
        let calls = store.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                "snapshot_state",
                "active_page",
                "active_session_ids",
                "active_head"
            ]
        );
        assert_eq!(payload.snapshot_rev, 17);
        assert_eq!(payload.archived_rev, 4);
        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(
            payload.tasks[0]
                .primary_session
                .last_message_preview
                .as_deref(),
            Some("canonical-summary")
        );
        assert_eq!(payload.tasks[0].primary_session.last_event_seq, Some(44));
        assert_eq!(payload.heads.len(), 1);
        assert_eq!(payload.heads[0].session.id, head.session.id);
        assert_eq!(payload.heads[0].last_event_seq, head.last_event_seq);
    }

    #[tokio::test]
    async fn workspace_hydration_payload_skips_bad_active_heads() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let healthy_session_id = SessionId::new();
        let failing_session_id = SessionId::new();
        let healthy_head = test_head(workspace_id, task_id, healthy_session_id);
        let store = FakeHydrationStore {
            snapshot_state: (19, 5),
            tasks: vec![test_summary(workspace_id, task_id, healthy_session_id)],
            session_ids: vec![healthy_session_id, failing_session_id],
            heads_by_session: HashMap::from([(healthy_session_id, healthy_head.clone())]),
            failing_heads: HashMap::from([(failing_session_id, "head decode failed")]),
            calls: Mutex::new(Vec::new()),
        };

        let payload = load_workspace_snapshot_hydration_payload(&store, workspace_id)
            .await
            .expect("expected hydration payload");

        assert_eq!(payload.snapshot_rev, 19);
        assert_eq!(payload.archived_rev, 5);
        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(payload.heads.len(), 1);
        assert_eq!(payload.heads[0].session.id, healthy_head.session.id);
    }

    #[tokio::test]
    async fn applying_workspace_hydration_payload_seeds_hub_with_loaded_snapshot_rev() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let hub = WorkspaceActiveSnapshotHub::new();
        let payload = WorkspaceSnapshotHydrationPayload {
            snapshot_rev: 23,
            archived_rev: 6,
            tasks: vec![test_summary(workspace_id, task_id, session_id)],
            heads: vec![test_head(workspace_id, task_id, session_id)],
        };

        apply_workspace_snapshot_hydration_payload(&hub, workspace_id, payload).await;

        let snapshot = hub.active_snapshot(workspace_id, i64::MAX).await;
        assert_eq!(snapshot.snapshot_rev, 23);
        assert_eq!(snapshot.archived_rev, 6);
        assert_eq!(snapshot.active.tasks.len(), 1);

        let heads = hub.active_heads(workspace_id).await;
        assert_eq!(heads.snapshot_rev, 23);
        assert_eq!(heads.heads.len(), 1);
        assert_eq!(heads.heads[0].session.id, session_id);
    }
}
