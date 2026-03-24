use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::Mutex;

use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;

use crate::Store;

mod leases;
use leases::WorkspaceStoreLeaseRegistry;
#[cfg(test)]
mod tests_eviction;

const DEFAULT_WORKSPACE_MAX_CONNECTIONS: u32 = 2;
const DEFAULT_MAX_CACHED_WORKSPACES: usize = 12;

#[derive(Clone, Debug)]
pub struct StoreManagerConfig {
    pub max_connections: Option<u32>,
    pub workspace_max_connections: Option<u32>,
    pub max_cached_workspaces: usize,
}

impl Default for StoreManagerConfig {
    fn default() -> Self {
        Self {
            max_connections: None,
            workspace_max_connections: None,
            max_cached_workspaces: DEFAULT_MAX_CACHED_WORKSPACES,
        }
    }
}

#[derive(Clone)]
pub struct StoreManager {
    global: Store,
    data_root: PathBuf,
    workspace_stores: Arc<Mutex<HashMap<WorkspaceId, TimedStoreEntry>>>,
    store_leases: Arc<WorkspaceStoreLeaseRegistry>,
    next_store_instance_id: Arc<AtomicU64>,
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
    workspace_id: WorkspaceId,
    store: Store,
    last_access: Instant,
    instance_id: u64,
}

pub struct WorkspaceStoreAccess {
    pub store: Store,
    pub opened_now: bool,
}

impl TimedStoreEntry {
    fn new(workspace_id: WorkspaceId, store: Store, instance_id: u64) -> Self {
        Self {
            workspace_id,
            store,
            last_access: Instant::now(),
            instance_id,
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
        mut config: StoreManagerConfig,
    ) -> Result<Self> {
        let data_root = data_root.as_ref().to_path_buf();
        let db_dir = data_root.join("db");
        tokio::fs::create_dir_all(&db_dir).await?;
        let global_db_path = db_dir.join("db.sqlite");
        config.max_cached_workspaces = config.max_cached_workspaces.max(1);
        let global = Store::open_sqlite(&global_db_path, config.max_connections).await?;

        Ok(Self {
            global,
            data_root,
            workspace_stores: Arc::new(Mutex::new(HashMap::new())),
            store_leases: Arc::new(WorkspaceStoreLeaseRegistry::default()),
            next_store_instance_id: Arc::new(AtomicU64::new(1)),
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
        Ok(self.workspace_access(workspace_id).await?.store)
    }

    pub async fn workspace_access(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceStoreAccess> {
        loop {
            loop {
                {
                    let mut stores = self.workspace_stores.lock().await;
                    if let Some(entry) = stores.get_mut(&workspace_id) {
                        entry.touch();
                        return Ok(WorkspaceStoreAccess {
                            store: entry.store.with_lease_guard(
                                self.store_leases.acquire(workspace_id, entry.instance_id),
                            ),
                            opened_now: false,
                        });
                    }
                }
                if let Some(store) = self.store_leases.acquire_pending_close_store(workspace_id) {
                    let workspace_exists = self.global.get_workspace(workspace_id).await?.is_some();
                    if workspace_exists {
                        return Ok(WorkspaceStoreAccess {
                            store,
                            opened_now: true,
                        });
                    }
                    drop(store);
                    self.store_leases
                        .wait_for_workspace_close(workspace_id)
                        .await;
                    continue;
                }
                let is_closing = self.store_leases.is_workspace_closing(workspace_id);
                if !is_closing {
                    break;
                }
                self.store_leases
                    .wait_for_workspace_close(workspace_id)
                    .await;
            }
            let workspace = self
                .global
                .get_workspace(workspace_id)
                .await?
                .with_context(|| format!("workspace {} not found", workspace_id.0))?;
            let store = self.open_workspace_store(&workspace).await?;
            let instance_id = self.next_store_instance_id.fetch_add(1, Ordering::Relaxed);
            let mut stores = self.workspace_stores.lock().await;
            if self.store_leases.is_workspace_closing(workspace_id) {
                drop(stores);
                store.close().await;
                self.store_leases
                    .wait_for_workspace_close(workspace_id)
                    .await;
                continue;
            }
            if let Some(existing) = stores.get_mut(&workspace_id) {
                existing.touch();
                let existing = existing.store.with_lease_guard(
                    self.store_leases
                        .acquire(workspace_id, existing.instance_id),
                );
                drop(stores);
                store.close().await;
                return Ok(WorkspaceStoreAccess {
                    store: existing,
                    opened_now: false,
                });
            }
            let leased_store =
                store.with_lease_guard(self.store_leases.acquire(workspace_id, instance_id));
            stores.insert(
                workspace_id,
                TimedStoreEntry::new(workspace_id, store.clone(), instance_id),
            );
            drop(stores);
            return Ok(WorkspaceStoreAccess {
                store: leased_store,
                opened_now: true,
            });
        }
    }

    pub async fn workspace_uncached(&self, workspace_id: WorkspaceId) -> Result<Store> {
        if self.store_leases.is_workspace_closing(workspace_id) {
            self.store_leases
                .wait_for_workspace_close(workspace_id)
                .await;
        }
        let workspace = self
            .global
            .get_workspace(workspace_id)
            .await?
            .with_context(|| format!("workspace {} not found", workspace_id.0))?;
        self.open_workspace_store(&workspace).await
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
            let close = stores.get(&workspace_id).and_then(|entry| {
                self.store_leases.queue_close(
                    entry.workspace_id,
                    entry.instance_id,
                    entry.store.clone(),
                )
            });
            stores.remove(&workspace_id);
            close
        };
        self.close_pending_entries(store.into_iter().collect())
            .await;
    }

    pub async fn evict_idle_workspaces(
        &self,
        max_idle: Duration,
        active_workspaces: &HashSet<WorkspaceId>,
    ) -> usize {
        let now = Instant::now();
        let (evicted, expired_entries) = {
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
            let evicted = expired.len();
            let closes = expired
                .into_iter()
                .filter_map(|workspace_id| {
                    let close = stores.get(&workspace_id).and_then(|entry| {
                        self.store_leases.queue_close(
                            entry.workspace_id,
                            entry.instance_id,
                            entry.store.clone(),
                        )
                    });
                    stores.remove(&workspace_id);
                    close
                })
                .collect::<Vec<_>>();
            (evicted, closes)
        };
        self.close_pending_entries(expired_entries).await;
        evicted
    }

    pub async fn evict_workspaces_to_cap(
        &self,
        protected_workspaces: &HashSet<WorkspaceId>,
    ) -> usize {
        let max_cached = self.config.max_cached_workspaces.max(1);
        let (evicted, expired_entries) = {
            let mut stores = self.workspace_stores.lock().await;
            let overflow = stores.len().saturating_sub(max_cached);
            if overflow == 0 {
                (0, Vec::new())
            } else {
                let mut victims = stores
                    .iter()
                    .filter(|(workspace_id, _)| !protected_workspaces.contains(workspace_id))
                    .map(|(workspace_id, entry)| (*workspace_id, entry.last_access))
                    .collect::<Vec<_>>();
                victims.sort_by_key(|(_, last_access)| *last_access);
                let victims = victims.into_iter().take(overflow).collect::<Vec<_>>();
                let evicted = victims.len();
                let closes = victims
                    .into_iter()
                    .filter_map(|(workspace_id, _)| {
                        let close = stores.get(&workspace_id).and_then(|entry| {
                            self.store_leases.queue_close(
                                entry.workspace_id,
                                entry.instance_id,
                                entry.store.clone(),
                            )
                        });
                        stores.remove(&workspace_id);
                        close
                    })
                    .collect::<Vec<_>>();
                (evicted, closes)
            }
        };
        self.close_pending_entries(expired_entries).await;
        evicted
    }

    async fn close_pending_entries(&self, entries: Vec<leases::PendingWorkspaceStoreClose>) {
        for close in entries {
            close.store.close().await;
            self.store_leases
                .finish_close(close.workspace_id, &close.notify);
        }
    }

    async fn open_workspace_store(&self, workspace: &Workspace) -> Result<Store> {
        let path = self.workspace_db_path(workspace.id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let workspace_max_connections = self
            .config
            .workspace_max_connections
            .or(self.config.max_connections)
            .or(Some(DEFAULT_WORKSPACE_MAX_CONNECTIONS));
        let store = Store::open_sqlite(&path, workspace_max_connections).await?;
        store.upsert_workspace(workspace).await?;
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
        assert_eq!(manager.stats().await.workspace_store_count, 0);

        let reopened = manager.workspace(workspace.id).await.unwrap();
        assert!(reopened
            .list_workspaces()
            .await
            .unwrap()
            .iter()
            .any(|candidate| candidate.id == workspace.id));
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
    async fn reopening_workspace_store_preserves_workspace_owned_records() {
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
        let reopened_store = manager.workspace(workspace.id).await.unwrap();

        assert!(reopened_store
            .get_artifact(artifact.id)
            .await
            .unwrap()
            .is_some());

        assert!(reopened_store
            .get_message(message.id)
            .await
            .unwrap()
            .is_some());

        assert!(reopened_store
            .get_subagent_invocation(&invocation.id)
            .await
            .unwrap()
            .is_some());

        assert!(reopened_store
            .get_merge_queue_entry(entry.id)
            .await
            .unwrap()
            .is_some());
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

    #[tokio::test]
    async fn workspace_uncached_does_not_populate_workspace_cache() {
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

        let store = manager.workspace_uncached(workspace.id).await.unwrap();
        assert_eq!(manager.stats().await.workspace_store_count, 0);

        store.close().await;
        assert_eq!(manager.stats().await.workspace_store_count, 0);
    }

    #[tokio::test]
    async fn workspace_access_reports_open_state_and_enforces_cache_cap() {
        let temp = tempfile::tempdir().unwrap();
        let manager = StoreManager::open_with_config(
            temp.path(),
            StoreManagerConfig {
                max_cached_workspaces: 2,
                ..StoreManagerConfig::default()
            },
        )
        .await
        .unwrap();
        let workspace_a = manager
            .global()
            .create_workspace(
                "a".to_string(),
                temp.path().join("a").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let workspace_b = manager
            .global()
            .create_workspace(
                "b".to_string(),
                temp.path().join("b").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let workspace_c = manager
            .global()
            .create_workspace(
                "c".to_string(),
                temp.path().join("c").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();

        assert!(
            manager
                .workspace_access(workspace_a.id)
                .await
                .unwrap()
                .opened_now
        );
        assert!(
            !manager
                .workspace_access(workspace_a.id)
                .await
                .unwrap()
                .opened_now
        );
        assert!(
            manager
                .workspace_access(workspace_b.id)
                .await
                .unwrap()
                .opened_now
        );
        assert_eq!(manager.stats().await.workspace_store_count, 2);

        assert!(
            manager
                .workspace_access(workspace_c.id)
                .await
                .unwrap()
                .opened_now
        );
        assert_eq!(manager.stats().await.workspace_store_count, 3);

        let evicted = manager
            .evict_workspaces_to_cap(&HashSet::from([workspace_c.id]))
            .await;
        assert_eq!(evicted, 1);
        assert_eq!(manager.stats().await.workspace_store_count, 2);

        assert!(
            manager
                .workspace_access(workspace_a.id)
                .await
                .unwrap()
                .opened_now
        );
        assert_eq!(manager.stats().await.workspace_store_count, 3);
    }

    #[tokio::test]
    async fn evict_workspaces_to_cap_preserves_protected_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let manager = StoreManager::open_with_config(
            temp.path(),
            StoreManagerConfig {
                max_cached_workspaces: 2,
                ..StoreManagerConfig::default()
            },
        )
        .await
        .unwrap();
        let workspace_a = manager
            .global()
            .create_workspace(
                "a".to_string(),
                temp.path().join("a").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let workspace_b = manager
            .global()
            .create_workspace(
                "b".to_string(),
                temp.path().join("b").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let workspace_c = manager
            .global()
            .create_workspace(
                "c".to_string(),
                temp.path().join("c").to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();

        let _ = manager.workspace_access(workspace_a.id).await.unwrap();
        let _ = manager.workspace_access(workspace_b.id).await.unwrap();
        let _ = manager.workspace_access(workspace_c.id).await.unwrap();
        assert_eq!(manager.stats().await.workspace_store_count, 3);

        let evicted = manager
            .evict_workspaces_to_cap(&HashSet::from([workspace_a.id]))
            .await;
        assert_eq!(evicted, 1);
        assert_eq!(manager.stats().await.workspace_store_count, 2);
        assert!(
            !manager
                .workspace_access(workspace_a.id)
                .await
                .unwrap()
                .opened_now
        );
        assert!(
            manager
                .workspace_access(workspace_b.id)
                .await
                .unwrap()
                .opened_now
        );
    }
}
