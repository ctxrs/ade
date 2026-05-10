use super::*;

impl AppState {
    pub fn global_store(&self) -> &Store {
        self.core.stores.global()
    }

    pub(in crate::daemon::state) async fn protected_workspace_store_ids(
        &self,
    ) -> HashSet<WorkspaceId> {
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
}
