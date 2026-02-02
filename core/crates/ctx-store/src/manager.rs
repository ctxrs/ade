use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;

use crate::Store;

#[derive(Clone, Debug, Default)]
pub struct StoreManagerConfig {
    pub max_connections: Option<u32>,
}

#[derive(Clone)]
pub struct StoreManager {
    global: Store,
    data_root: PathBuf,
    global_db_path: PathBuf,
    workspace_stores: Arc<Mutex<HashMap<WorkspaceId, TimedStoreEntry>>>,
    config: StoreManagerConfig,
}

#[derive(Clone)]
struct TimedStoreEntry {
    store: Store,
    last_access: Instant,
}

impl TimedStoreEntry {
    fn new(store: Store) -> Self {
        Self {
            store,
            last_access: Instant::now(),
        }
    }

    fn touch(&mut self) {
        self.last_access = Instant::now();
    }
}

impl StoreManager {
    pub async fn open(data_root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_config(data_root, StoreManagerConfig::default()).await
    }

    pub async fn open_with_config(
        data_root: impl AsRef<Path>,
        config: StoreManagerConfig,
    ) -> Result<Self> {
        let data_root = data_root.as_ref().to_path_buf();
        let db_dir = data_root.join("db");
        tokio::fs::create_dir_all(&db_dir).await?;
        let global_db_path = db_dir.join("db.sqlite");
        let legacy_db_path = data_root.join("db.sqlite");
        if legacy_db_path.exists() && !global_db_path.exists() {
            tokio::fs::rename(&legacy_db_path, &global_db_path).await?;
            let legacy_wal = legacy_db_path.with_extension("sqlite-wal");
            let legacy_shm = legacy_db_path.with_extension("sqlite-shm");
            let global_wal = global_db_path.with_extension("sqlite-wal");
            let global_shm = global_db_path.with_extension("sqlite-shm");
            if legacy_wal.exists() {
                tokio::fs::rename(&legacy_wal, &global_wal).await?;
            }
            if legacy_shm.exists() {
                tokio::fs::rename(&legacy_shm, &global_shm).await?;
            }
        }

        let global = Store::open_sqlite(&global_db_path, config.max_connections).await?;

        let manager = Self {
            global,
            data_root,
            global_db_path,
            workspace_stores: Arc::new(Mutex::new(HashMap::new())),
            config,
        };
        manager.bootstrap_workspace_dbs().await?;
        Ok(manager)
    }

    pub fn global(&self) -> &Store {
        &self.global
    }

    pub async fn workspace(&self, workspace_id: WorkspaceId) -> Result<Store> {
        {
            let mut stores = self.workspace_stores.lock().await;
            if let Some(entry) = stores.get_mut(&workspace_id) {
                entry.touch();
                return Ok(entry.store.clone());
            }
        }
        let workspace = self
            .global
            .get_workspace(workspace_id)
            .await?
            .with_context(|| format!("workspace {} not found", workspace_id.0))?;
        let store = self.open_workspace_store(&workspace, false).await?;
        let mut stores = self.workspace_stores.lock().await;
        if let Some(existing) = stores.get_mut(&workspace_id) {
            existing.touch();
            let existing = existing.store.clone();
            drop(stores);
            store.close().await;
            return Ok(existing);
        }
        stores.insert(workspace_id, TimedStoreEntry::new(store.clone()));
        Ok(store)
    }

    pub async fn store_for_task(&self, task_id: TaskId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_task(task_id)
            .await?
            .with_context(|| format!("workspace missing for task {}", task_id.0))?;
        self.workspace(workspace_id).await
    }

    pub async fn store_for_session(&self, session_id: SessionId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_session(session_id)
            .await?
            .with_context(|| format!("workspace missing for session {}", session_id.0))?;
        self.workspace(workspace_id).await
    }

    pub async fn store_for_worktree(&self, worktree_id: WorktreeId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_worktree(worktree_id)
            .await?
            .with_context(|| format!("workspace missing for worktree {}", worktree_id.0))?;
        self.workspace(workspace_id).await
    }

    pub async fn evict_workspace(&self, workspace_id: WorkspaceId) {
        let store = {
            let mut stores = self.workspace_stores.lock().await;
            stores.remove(&workspace_id)
        };
        if let Some(store) = store {
            store.store.close().await;
        }
    }

    pub async fn evict_idle_workspaces(
        &self,
        max_idle: Duration,
        active_workspaces: &HashSet<WorkspaceId>,
    ) -> usize {
        let now = Instant::now();
        let evicted = {
            let mut stores = self.workspace_stores.lock().await;
            let expired: Vec<WorkspaceId> = stores
                .iter()
                .filter_map(|(workspace_id, entry)| {
                    if active_workspaces.contains(workspace_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= max_idle {
                        Some(*workspace_id)
                    } else {
                        None
                    }
                })
                .collect();
            for workspace_id in &expired {
                stores.remove(workspace_id);
            }
            expired.len()
        };
        evicted
    }

    async fn bootstrap_workspace_dbs(&self) -> Result<()> {
        let workspaces = self.global.list_workspaces().await?;
        for workspace in workspaces {
            self.global.refresh_workspace_indexes(workspace.id).await?;
            let _ = self.open_workspace_store(&workspace, true).await?;
        }
        Ok(())
    }

    async fn open_workspace_store(
        &self,
        workspace: &Workspace,
        migrate_if_missing: bool,
    ) -> Result<Store> {
        let path = self.workspace_db_path(workspace.id);
        let needs_migration = !path.exists();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let store = Store::open_sqlite(&path, self.config.max_connections).await?;
        store.upsert_workspace(workspace).await?;
        if migrate_if_missing && needs_migration {
            if let Err(err) = store
                .migrate_workspace_from_path(&self.global_db_path, workspace.id)
                .await
            {
                store.close().await;
                let _ = tokio::fs::remove_file(&path).await;
                return Err(err);
            }
        }
        Ok(store)
    }

    fn workspace_db_path(&self, workspace_id: WorkspaceId) -> PathBuf {
        self.data_root
            .join("db")
            .join("workspaces")
            .join(workspace_id.0.to_string())
            .join("db.sqlite")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::models::VcsKind;

    #[tokio::test]
    async fn evict_idle_workspaces_skips_active() {
        let temp = tempfile::tempdir().unwrap();
        let manager = StoreManager::open(temp.path()).await.unwrap();
        let workspace = manager
            .global()
            .create_workspace(
                "ws".to_string(),
                temp.path().to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let _store = manager.workspace(workspace.id).await.unwrap();

        let mut active = HashSet::new();
        active.insert(workspace.id);
        let evicted = manager
            .evict_idle_workspaces(Duration::from_secs(0), &active)
            .await;
        assert_eq!(evicted, 0);

        let evicted = manager
            .evict_idle_workspaces(Duration::from_secs(0), &HashSet::new())
            .await;
        assert_eq!(evicted, 1);
    }
}
