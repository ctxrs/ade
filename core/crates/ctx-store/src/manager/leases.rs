use super::*;

use std::collections::HashMap;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{anyhow, Result};
use tokio::sync::{mpsc as tokio_mpsc, Notify};

pub(super) struct WorkspaceStoreLeaseRegistry {
    state: StdMutex<WorkspaceStoreLeaseState>,
    close_executor: StoreCloseExecutor,
}

#[derive(Default)]
struct WorkspaceStoreLeaseState {
    entries: HashMap<u64, WorkspaceStoreLeaseEntry>,
    closing_workspaces: HashMap<WorkspaceId, Arc<Notify>>,
}

struct WorkspaceStoreLeaseEntry {
    workspace_id: WorkspaceId,
    in_use: usize,
    pending_close: Option<Store>,
}

struct WorkspaceStoreLease {
    instance_id: u64,
    registry: Arc<WorkspaceStoreLeaseRegistry>,
}

pub(super) struct PendingWorkspaceStoreClose {
    pub(super) workspace_id: WorkspaceId,
    pub(super) store: Store,
    pub(super) notify: Arc<Notify>,
}

pub(super) struct ReactivatedWorkspaceStore {
    pub(super) store: Store,
    pub(super) instance_id: u64,
    pub(super) notify: Arc<Notify>,
}

struct StoreCloseJob {
    store: Store,
    registry: Arc<WorkspaceStoreLeaseRegistry>,
    workspace_id: WorkspaceId,
    notify: Arc<Notify>,
}

struct StoreCloseExecutor {
    tx: tokio_mpsc::UnboundedSender<StoreCloseJob>,
}

impl StoreCloseExecutor {
    fn new() -> Result<Self> {
        let (tx, mut rx) = tokio_mpsc::unbounded_channel::<StoreCloseJob>();
        let (ready_tx, ready_rx) = sync_channel::<Result<()>>(1);
        std::thread::Builder::new()
            .name("ctx-store-close-executor".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|err| anyhow!("failed to build store close runtime: {err}"));
                match runtime {
                    Ok(runtime) => {
                        let _ = ready_tx.send(Ok(()));
                        runtime.block_on(async move {
                            let mut closes = tokio::task::JoinSet::new();
                            loop {
                                tokio::select! {
                                    maybe_job = rx.recv() => {
                                        match maybe_job {
                                            Some(job) => {
                                                closes.spawn(close_store_and_finish(
                                                    job.store,
                                                    Arc::clone(&job.registry),
                                                    job.workspace_id,
                                                    Arc::clone(&job.notify),
                                                ));
                                            }
                                            None => break,
                                        }
                                    }
                                    result = closes.join_next(), if !closes.is_empty() => {
                                        if let Some(Err(err)) = result {
                                            tracing::warn!("store close task failed: {err:#}");
                                        }
                                    }
                                }
                            }
                            while let Some(result) = closes.join_next().await {
                                if let Err(err) = result {
                                    tracing::warn!("store close task failed: {err:#}");
                                }
                            }
                        });
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                    }
                }
            })
            .map_err(|err| anyhow!("failed to spawn store close executor thread: {err}"))?;
        ready_rx
            .recv()
            .map_err(|err| anyhow!("store close executor startup failed: {err}"))??;
        Ok(Self { tx })
    }

    fn submit(&self, job: StoreCloseJob) -> Result<()> {
        self.tx
            .send(job)
            .map_err(|_| anyhow!("store close executor is not available"))
    }
}

impl WorkspaceStoreLeaseRegistry {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            state: StdMutex::new(WorkspaceStoreLeaseState::default()),
            close_executor: StoreCloseExecutor::new()?,
        })
    }

    pub(super) fn acquire(
        self: &Arc<Self>,
        workspace_id: WorkspaceId,
        instance_id: u64,
    ) -> Arc<dyn crate::store::StoreLeaseGuard> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = state
            .entries
            .entry(instance_id)
            .or_insert_with(|| WorkspaceStoreLeaseEntry {
                workspace_id,
                in_use: 0,
                pending_close: None,
            });
        entry.workspace_id = workspace_id;
        entry.in_use += 1;
        Arc::new(WorkspaceStoreLease {
            instance_id,
            registry: Arc::clone(self),
        })
    }

    pub(super) async fn wait_for_workspace_close(&self, workspace_id: WorkspaceId) {
        loop {
            let wait = {
                let state = match self.state.lock() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                state
                    .closing_workspaces
                    .get(&workspace_id)
                    .cloned()
                    .map(|notify| notify.notified_owned())
            };
            match wait {
                Some(wait) => wait.await,
                None => return,
            }
        }
    }

    pub(super) fn is_workspace_closing(&self, workspace_id: WorkspaceId) -> bool {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.closing_workspaces.contains_key(&workspace_id)
    }

    pub(super) fn has_pending_close_store(&self, workspace_id: WorkspaceId) -> bool {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state
            .entries
            .values()
            .any(|entry| entry.workspace_id == workspace_id && entry.pending_close.is_some())
    }

    pub(super) fn reactivate_pending_close_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<ReactivatedWorkspaceStore> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let instance_id = state.entries.iter().find_map(|(instance_id, entry)| {
            (entry.workspace_id == workspace_id && entry.pending_close.is_some())
                .then_some(*instance_id)
        })?;
        let store = state.entries.get_mut(&instance_id)?.pending_close.take()?;
        let notify = state.closing_workspaces.get(&workspace_id)?.clone();
        Some(ReactivatedWorkspaceStore {
            store,
            instance_id,
            notify,
        })
    }

    pub(super) fn publish_reactivated_store(
        &self,
        workspace_id: WorkspaceId,
        notify: &Arc<Notify>,
    ) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        clear_closing_marker(&mut state, workspace_id, notify);
    }

    pub(super) fn restore_pending_close_store(
        &self,
        workspace_id: WorkspaceId,
        instance_id: u64,
        store: Store,
    ) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = state
            .entries
            .entry(instance_id)
            .or_insert_with(|| WorkspaceStoreLeaseEntry {
                workspace_id,
                in_use: 0,
                pending_close: None,
            });
        entry.workspace_id = workspace_id;
        entry.pending_close = Some(store);
        let _ = closing_notify(&mut state, workspace_id);
    }

    pub(super) fn queue_close(
        &self,
        workspace_id: WorkspaceId,
        instance_id: u64,
        store: Store,
    ) -> Option<PendingWorkspaceStoreClose> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let close_now = state
            .entries
            .get(&instance_id)
            .map(|entry| entry.in_use == 0)
            .unwrap_or(true);
        if close_now {
            state.entries.remove(&instance_id);
            Some(PendingWorkspaceStoreClose {
                workspace_id,
                store,
                notify: closing_notify(&mut state, workspace_id),
            })
        } else {
            let entry =
                state
                    .entries
                    .entry(instance_id)
                    .or_insert_with(|| WorkspaceStoreLeaseEntry {
                        workspace_id,
                        in_use: 0,
                        pending_close: None,
                    });
            entry.workspace_id = workspace_id;
            entry.pending_close = Some(store);
            let _ = closing_notify(&mut state, workspace_id);
            None
        }
    }

    fn release(&self, instance_id: u64) -> Option<PendingWorkspaceStoreClose> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let (should_remove, workspace_id, pending_close) = {
            let entry = state.entries.get_mut(&instance_id)?;
            entry.in_use = entry.in_use.saturating_sub(1);
            if entry.in_use == 0 {
                (true, entry.workspace_id, entry.pending_close.take())
            } else {
                (false, entry.workspace_id, None)
            }
        };
        if should_remove {
            state.entries.remove(&instance_id);
            pending_close.map(|store| PendingWorkspaceStoreClose {
                workspace_id,
                store,
                notify: closing_notify(&mut state, workspace_id),
            })
        } else {
            None
        }
    }

    pub(super) fn finish_close(&self, workspace_id: WorkspaceId, notify: &Arc<Notify>) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        clear_closing_marker(&mut state, workspace_id, notify);
    }
}

fn closing_notify(state: &mut WorkspaceStoreLeaseState, workspace_id: WorkspaceId) -> Arc<Notify> {
    state
        .closing_workspaces
        .entry(workspace_id)
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

fn clear_closing_marker(
    state: &mut WorkspaceStoreLeaseState,
    workspace_id: WorkspaceId,
    notify: &Arc<Notify>,
) {
    if state
        .closing_workspaces
        .get(&workspace_id)
        .is_some_and(|current| Arc::ptr_eq(current, notify))
    {
        state.closing_workspaces.remove(&workspace_id);
    }
    notify.notify_waiters();
}

impl Drop for WorkspaceStoreLease {
    fn drop(&mut self) {
        if let Some(close) = self.registry.release(self.instance_id) {
            spawn_store_close(
                close.store,
                Arc::clone(&self.registry),
                close.workspace_id,
                close.notify,
            );
        }
    }
}

fn spawn_store_close(
    store: Store,
    registry: Arc<WorkspaceStoreLeaseRegistry>,
    workspace_id: WorkspaceId,
    notify: Arc<Notify>,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(close_store_and_finish(
            store,
            Arc::clone(&registry),
            workspace_id,
            Arc::clone(&notify),
        ));
        return;
    }

    if let Err(err) = registry.close_executor.submit(StoreCloseJob {
        store: store.clone(),
        registry: Arc::clone(&registry),
        workspace_id,
        notify: Arc::clone(&notify),
    }) {
        tracing::warn!(
            workspace_id = %workspace_id.0,
            "failed to submit workspace close to executor: {err:#}"
        );
        store.close_blocking();
        registry.finish_close(workspace_id, &notify);
    }
}

async fn close_store_and_finish(
    store: Store,
    registry: Arc<WorkspaceStoreLeaseRegistry>,
    workspace_id: WorkspaceId,
    notify: Arc<Notify>,
) {
    store.close().await;
    registry.finish_close(workspace_id, &notify);
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    async fn open_test_store(temp: &tempfile::TempDir, name: &str) -> Store {
        let path = temp.path().join(name);
        Store::open_sqlite(&path, Some(1)).await.unwrap()
    }

    #[tokio::test]
    async fn immediate_close_registers_workspace_as_closing() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
        let workspace_id = WorkspaceId::new();
        let store = open_test_store(&temp, "immediate-close.sqlite").await;

        let close = registry
            .queue_close(workspace_id, 1, store)
            .expect("close without leases should start immediately");

        let blocked = tokio::time::timeout(
            Duration::from_millis(20),
            registry.wait_for_workspace_close(workspace_id),
        )
        .await;
        assert!(
            blocked.is_err(),
            "waiters must observe an in-progress immediate close"
        );

        close.store.close().await;
        registry.finish_close(workspace_id, &close.notify);

        tokio::time::timeout(
            Duration::from_secs(1),
            registry.wait_for_workspace_close(workspace_id),
        )
        .await
        .expect("waiter should be released after close completes");
    }

    #[tokio::test]
    async fn waiters_do_not_miss_close_notifications() {
        let temp = tempfile::tempdir().unwrap();

        for idx in 0..64 {
            let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
            let workspace_id = WorkspaceId::new();
            let store = open_test_store(&temp, &format!("close-race-{idx}.sqlite")).await;
            let close = registry
                .queue_close(workspace_id, idx + 1, store)
                .expect("close without leases should start immediately");
            let waiter_registry = Arc::clone(&registry);
            let waiter = tokio::spawn(async move {
                tokio::time::timeout(
                    Duration::from_secs(1),
                    waiter_registry.wait_for_workspace_close(workspace_id),
                )
                .await
            });

            tokio::task::yield_now().await;
            close.store.close().await;
            registry.finish_close(workspace_id, &close.notify);

            assert!(
                waiter.await.unwrap().is_ok(),
                "waiter should not miss the close notification"
            );
        }
    }

    #[tokio::test]
    async fn pending_close_marks_workspace_as_closing_before_last_lease_drops() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
        let workspace_id = WorkspaceId::new();
        let lease = registry.acquire(workspace_id, 7);
        let store = open_test_store(&temp, "pending-close.sqlite").await;

        assert!(
            registry.queue_close(workspace_id, 7, store).is_none(),
            "active lease should defer the close"
        );

        let blocked = tokio::time::timeout(
            Duration::from_millis(20),
            registry.wait_for_workspace_close(workspace_id),
        )
        .await;
        assert!(
            blocked.is_err(),
            "reopens must wait even while the last lease is still draining"
        );

        drop(lease);

        tokio::time::timeout(
            Duration::from_secs(1),
            registry.wait_for_workspace_close(workspace_id),
        )
        .await
        .expect("waiter should be released once deferred close completes");
    }

    #[tokio::test]
    async fn pending_close_store_can_be_reacquired_without_deadlock() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
        let workspace_id = WorkspaceId::new();
        let lease = registry.acquire(workspace_id, 9);
        let store = open_test_store(&temp, "pending-reacquire.sqlite").await;

        assert!(
            registry.queue_close(workspace_id, 9, store).is_none(),
            "active lease should defer the close"
        );

        let reopened = registry
            .reactivate_pending_close_store(workspace_id)
            .expect("deferred close should reactivate the draining store");
        assert!(
            registry.is_workspace_closing(workspace_id),
            "reactivation should keep the closing marker until publication"
        );
        registry.publish_reactivated_store(workspace_id, &reopened.notify);
        drop(reopened.store);
        drop(lease);

        assert!(
            !registry.is_workspace_closing(workspace_id),
            "publishing the reactivated store should cancel the deferred close"
        );
    }

    #[test]
    fn close_without_runtime_completes_inline() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let store = runtime.block_on(open_test_store(&temp, "inline-close.sqlite"));
        drop(runtime);

        let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
        let workspace_id = WorkspaceId::new();
        let close = registry
            .queue_close(workspace_id, 11, store)
            .expect("close without leases should start immediately");

        spawn_store_close(
            close.store,
            Arc::clone(&registry),
            close.workspace_id,
            Arc::clone(&close.notify),
        );

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(1),
                registry.wait_for_workspace_close(workspace_id),
            )
            .await
            .expect("inline close should finish without a background runtime");
        });

        assert!(
            !registry.is_workspace_closing(workspace_id),
            "inline close should clear the closing marker once shutdown completes"
        );
    }

    #[test]
    fn close_without_runtime_executor_path_clears_closing_marker() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let store = runtime.block_on(open_test_store(&temp, "inline-close-executor.sqlite"));
        drop(runtime);

        let registry = Arc::new(WorkspaceStoreLeaseRegistry::new().unwrap());
        let workspace_id = WorkspaceId::new();
        let close = registry
            .queue_close(workspace_id, 12, store)
            .expect("close without leases should start immediately");

        spawn_store_close(
            close.store,
            Arc::clone(&registry),
            close.workspace_id,
            Arc::clone(&close.notify),
        );

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(1),
                registry.wait_for_workspace_close(workspace_id),
            )
            .await
            .expect("executor-backed close should still release the closing marker");
        });

        assert!(
            !registry.is_workspace_closing(workspace_id),
            "executor-backed close should clear the closing marker after dropping the final store handle"
        );
    }
}
