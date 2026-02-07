use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Task, TaskDeltaKind, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree,
    WorktreeVcsSnapshot,
};
use ctx_lsp::Language as LspLanguage;

use crate::git_status;

use super::state::{
    AppState, TimedEntry, WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
    WorkspaceRuntime, WorktreeBootstrapGate,
};

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
        let (_, archived_rev) = match store
            .get_workspace_active_snapshot_state(workspace_id)
            .await
        {
            Ok(state) => state,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot state"
                );
                return;
            }
        };
        let (tasks, _) = match store
            .list_workspace_active_page_base(workspace_id, i64::MAX)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot tasks"
                );
                return;
            }
        };
        let session_ids = match store.list_workspace_active_session_ids(workspace_id).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    workspace_id = ?workspace_id,
                    err = %err,
                    "failed to hydrate workspace snapshot session ids"
                );
                return;
            }
        };
        let mut heads = Vec::new();
        for session_id in session_ids {
            if let Ok(Some(head)) = store.get_active_snapshot_head(session_id).await {
                heads.push(head);
            }
        }
        self.workspace_active_snapshot
            .hydrate_snapshot(workspace_id, 0, archived_rev, tasks, heads)
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
