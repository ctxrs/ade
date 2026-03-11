use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::Mutex;

use ctx_core::ids::{
    ArtifactId, MergeQueueEntryId, MessageId, SessionId, TaskId, WorkspaceId, WorktreeId,
};
use ctx_core::models::Workspace;
use tracing::warn;

use crate::Store;

#[derive(Clone, Debug, Default)]
pub struct StoreManagerConfig {
    pub max_connections: Option<u32>,
}

#[derive(Clone)]
pub struct StoreManager {
    global: Store,
    data_root: PathBuf,
    workspace_stores: Arc<Mutex<HashMap<WorkspaceId, TimedStoreEntry>>>,
    config: StoreManagerConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoreManagerStats {
    pub global_pool_size: usize,
    pub global_pool_idle: usize,
    pub workspace_store_count: usize,
    pub workspace_pool_size_total: usize,
    pub workspace_pool_idle_total: usize,
    pub workspace_pool_size_max: usize,
    pub workspace_pool_idle_max: usize,
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
        let global = Store::open_sqlite(&global_db_path, config.max_connections).await?;

        Ok(Self {
            global,
            data_root,
            workspace_stores: Arc::new(Mutex::new(HashMap::new())),
            config,
        })
    }

    pub fn global(&self) -> &Store {
        &self.global
    }

    pub async fn stats(&self) -> StoreManagerStats {
        let global_stats = self.global.stats();
        let stores = self.workspace_stores.lock().await;
        let workspace_store_count = stores.len();
        let mut workspace_pool_size_total: usize = 0;
        let mut workspace_pool_idle_total: usize = 0;
        let mut workspace_pool_size_max: usize = 0;
        let mut workspace_pool_idle_max: usize = 0;
        for entry in stores.values() {
            let stats = entry.store.stats();
            workspace_pool_size_total = workspace_pool_size_total.saturating_add(stats.pool_size);
            workspace_pool_idle_total = workspace_pool_idle_total.saturating_add(stats.pool_idle);
            if stats.pool_size > workspace_pool_size_max {
                workspace_pool_size_max = stats.pool_size;
            }
            if stats.pool_idle > workspace_pool_idle_max {
                workspace_pool_idle_max = stats.pool_idle;
            }
        }
        StoreManagerStats {
            global_pool_size: global_stats.pool_size,
            global_pool_idle: global_stats.pool_idle,
            workspace_store_count,
            workspace_pool_size_total,
            workspace_pool_idle_total,
            workspace_pool_size_max,
            workspace_pool_idle_max,
        }
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
        let store = self.open_workspace_store(&workspace).await?;
        if let Err(err) = self
            .sync_workspace_global_entity_routing_indexes(workspace_id, &store)
            .await
        {
            warn!(workspace_id = %workspace_id.0, "failed to refresh global routing indexes: {err:#}");
        }
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

    pub async fn store_for_artifact(&self, artifact_id: ArtifactId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_artifact(artifact_id)
            .await?
            .with_context(|| format!("workspace missing for artifact {}", artifact_id.0))?;
        self.workspace(workspace_id).await
    }

    pub async fn store_for_message(&self, message_id: MessageId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_message(message_id)
            .await?
            .with_context(|| format!("workspace missing for message {}", message_id.0))?;
        self.workspace(workspace_id).await
    }

    pub async fn store_for_subagent_invocation(&self, invocation_id: &str) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_subagent_invocation(invocation_id)
            .await?
            .with_context(|| {
                format!("workspace missing for subagent invocation {invocation_id}")
            })?;
        self.workspace(workspace_id).await
    }

    pub async fn store_for_merge_queue_entry(&self, entry_id: MergeQueueEntryId) -> Result<Store> {
        let workspace_id = self
            .global
            .get_workspace_id_for_merge_queue_entry(entry_id)
            .await?
            .with_context(|| format!("workspace missing for merge queue entry {}", entry_id.0))?;
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

    async fn open_workspace_store(&self, workspace: &Workspace) -> Result<Store> {
        let path = self.workspace_db_path(workspace.id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let store = Store::open_sqlite(&path, self.config.max_connections).await?;
        store.upsert_workspace(workspace).await?;
        Ok(store)
    }

    async fn sync_workspace_global_entity_routing_indexes(
        &self,
        workspace_id: WorkspaceId,
        store: &Store,
    ) -> Result<()> {
        let artifact_ids = store.list_artifact_ids().await?;
        self.global
            .replace_workspace_artifact_index(workspace_id, &artifact_ids)
            .await?;

        let message_ids = store.list_message_ids().await?;
        self.global
            .replace_workspace_message_index(workspace_id, &message_ids)
            .await?;

        let invocation_ids = store.list_subagent_invocation_ids().await?;
        self.global
            .replace_workspace_subagent_invocation_index(workspace_id, &invocation_ids)
            .await?;

        let merge_queue_entries = store.list_merge_queue_entry_index_records().await?;
        self.global
            .replace_workspace_merge_queue_entry_index(workspace_id, &merge_queue_entries)
            .await?;

        Ok(())
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
    use chrono::Utc;
    use ctx_core::ids::{ArtifactId, MergeQueueEntryId, MessageId};
    use ctx_core::models::VcsKind;
    use ctx_core::models::{
        Artifact, MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, Message,
        MessageDelivery, MessageRole, SubagentInvocation,
    };
    use std::fs;

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

    #[tokio::test]
    async fn startup_uses_split_layout_without_legacy_root_migration() {
        let temp = tempfile::tempdir().unwrap();
        let manager = StoreManager::open(temp.path()).await.unwrap();
        let split_global = temp.path().join("db").join("db.sqlite");
        assert!(split_global.exists(), "expected split-layout global db");
        drop(manager);

        let legacy_root_db = temp.path().join("db.sqlite");
        fs::write(&legacy_root_db, b"legacy-data").unwrap();
        assert!(legacy_root_db.exists());

        let _manager = StoreManager::open(temp.path()).await.unwrap();
        assert!(
            legacy_root_db.exists(),
            "legacy root db must be ignored, not renamed"
        );
        let legacy_bytes = fs::read(&legacy_root_db).unwrap();
        assert_eq!(legacy_bytes, b"legacy-data");
    }

    #[tokio::test]
    async fn reopening_workspace_store_backfills_global_entity_routes() {
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
        let store = manager.workspace(workspace.id).await.unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                temp.path().to_string_lossy().to_string(),
                "base".to_string(),
                None,
            )
            .await
            .unwrap();
        let task = store
            .create_task(workspace.id, "task".to_string(), None)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                ctx_core::models::ExecutionEnvironment::Host,
                "fake".to_string(),
                "fake-model".to_string(),
                "assistant".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let artifact = Artifact {
            id: ArtifactId::new(),
            session_id: session.id,
            task_id: task.id,
            workspace_id: workspace.id,
            worktree_id: worktree.id,
            name: Some("artifact.txt".to_string()),
            absolute_path: temp
                .path()
                .join("artifact.txt")
                .to_string_lossy()
                .to_string(),
            mime_type: "text/plain".to_string(),
            bytes: 4,
            missing: None,
            created_at: Utc::now(),
        };
        std::fs::write(&artifact.absolute_path, "test").unwrap();
        store
            .replace_session_artifacts(session.id, std::slice::from_ref(&artifact))
            .await
            .unwrap();

        let message = store
            .insert_message(Message {
                id: MessageId::new(),
                session_id: session.id,
                task_id: task.id,
                run_id: None,
                turn_id: None,
                turn_sequence: None,
                order_seq: None,
                role: MessageRole::User,
                content: "queued".to_string(),
                attachments: Vec::new(),
                delivery: MessageDelivery::Queued,
                delivered_at: None,
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let invocation = store
            .upsert_subagent_invocation(SubagentInvocation {
                id: "subagent-test".to_string(),
                tool_call_id: "tool-call".to_string(),
                parent_session_id: session.id,
                parent_turn_id: None,
                requested_count: 1,
                request_json: None,
                status: "running".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                children: Vec::new(),
            })
            .await
            .unwrap();

        let entry = MergeQueueEntry {
            id: MergeQueueEntryId::new(),
            workspace_id: workspace.id,
            worktree_id: Some(worktree.id),
            session_id: Some(session.id),
            target_branch: "main".to_string(),
            message: Some("merge me".to_string()),
            patch_source: MergeQueuePatchSource::Generated,
            base_commit_sha: Some("base".to_string()),
            head_commit_sha: Some("head".to_string()),
            patch_path: temp.path().join("patch.diff").to_string_lossy().to_string(),
            patch_size: 12,
            status: MergeQueueEntryStatus::Queued,
            result_commit_sha: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.create_merge_queue_entry(&entry).await.unwrap();

        manager.evict_workspace(workspace.id).await;
        let _store = manager.workspace(workspace.id).await.unwrap();

        let artifact_store = manager.store_for_artifact(artifact.id).await.unwrap();
        assert!(artifact_store
            .get_artifact(artifact.id)
            .await
            .unwrap()
            .is_some());

        let message_store = manager.store_for_message(message.id).await.unwrap();
        assert!(message_store
            .get_message(message.id)
            .await
            .unwrap()
            .is_some());

        let invocation_store = manager
            .store_for_subagent_invocation(&invocation.id)
            .await
            .unwrap();
        assert!(invocation_store
            .get_subagent_invocation(&invocation.id)
            .await
            .unwrap()
            .is_some());

        let entry_store = manager.store_for_merge_queue_entry(entry.id).await.unwrap();
        assert!(entry_store
            .get_merge_queue_entry(entry.id)
            .await
            .unwrap()
            .is_some());

        let queued = manager
            .global()
            .list_queued_merge_queue_entry_routes()
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].entry_id.0, entry.id.0);
        assert_eq!(queued[0].workspace_id.0, workspace.id.0);
    }

    #[tokio::test]
    async fn startup_does_not_bootstrap_workspace_dbs() {
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
        let workspace_db_path = temp
            .path()
            .join("db")
            .join("workspaces")
            .join(workspace.id.0.to_string())
            .join("db.sqlite");
        assert!(!workspace_db_path.exists());
        drop(manager);

        let manager = StoreManager::open(temp.path()).await.unwrap();
        assert!(
            !workspace_db_path.exists(),
            "workspace db should not be created until first workspace access"
        );

        let _store = manager.workspace(workspace.id).await.unwrap();
        assert!(workspace_db_path.exists());
    }

    #[tokio::test]
    async fn workspace_open_does_not_import_rows_from_global_db() {
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
        let global_task = manager
            .global()
            .create_task(workspace.id, "legacy-task".to_string(), None)
            .await
            .unwrap();
        assert_eq!(
            manager
                .global()
                .list_tasks(workspace.id)
                .await
                .unwrap()
                .len(),
            1,
            "global db task setup failed"
        );

        let workspace_store = manager.workspace(workspace.id).await.unwrap();
        let workspace_tasks = workspace_store.list_tasks(workspace.id).await.unwrap();
        assert!(
            workspace_tasks.is_empty(),
            "workspace open must not import rows"
        );
        assert!(
            workspace_store
                .get_task(global_task.id)
                .await
                .unwrap()
                .is_none(),
            "global task id must not appear in workspace db"
        );
    }
}
