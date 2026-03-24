use super::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct WorkspaceStoreLeaseRegistry {
    state: StdMutex<WorkspaceStoreLeaseState>,
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

impl WorkspaceStoreLeaseRegistry {
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

    pub(super) fn acquire_pending_close_store(
        self: &Arc<Self>,
        workspace_id: WorkspaceId,
    ) -> Option<Store> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let (instance_id, store) = state.entries.iter().find_map(|(instance_id, entry)| {
            (entry.workspace_id == workspace_id)
                .then(|| {
                    entry
                        .pending_close
                        .as_ref()
                        .cloned()
                        .map(|store| (*instance_id, store))
                })
                .flatten()
        })?;
        let entry = state.entries.get_mut(&instance_id)?;
        entry.in_use += 1;
        Some(store.with_lease_guard(Arc::new(WorkspaceStoreLease {
            instance_id,
            registry: Arc::clone(self),
        })))
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
        if state
            .closing_workspaces
            .get(&workspace_id)
            .is_some_and(|current| Arc::ptr_eq(current, notify))
        {
            state.closing_workspaces.remove(&workspace_id);
        }
        notify.notify_waiters();
    }
}

fn closing_notify(state: &mut WorkspaceStoreLeaseState, workspace_id: WorkspaceId) -> Arc<Notify> {
    state
        .closing_workspaces
        .entry(workspace_id)
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
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
    let close_registry = Arc::clone(&registry);
    let close_notify = Arc::clone(&notify);
    let close_task = async move {
        store.close().await;
        close_registry.finish_close(workspace_id, &close_notify);
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(close_task);
        return;
    }
    let thread_registry = Arc::clone(&registry);
    let thread_notify = Arc::clone(&notify);
    if let Err(err) = std::thread::Builder::new()
        .name("ctx-store-close".to_string())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Runtime::new() {
                runtime.block_on(close_task);
            } else {
                thread_registry.finish_close(workspace_id, &thread_notify);
            }
        })
    {
        tracing::warn!(
            workspace_id = %workspace_id.0,
            "failed to spawn workspace close thread: {err}"
        );
        registry.finish_close(workspace_id, &notify);
    }
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
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::default());
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
            let registry = Arc::new(WorkspaceStoreLeaseRegistry::default());
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
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::default());
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
        let registry = Arc::new(WorkspaceStoreLeaseRegistry::default());
        let workspace_id = WorkspaceId::new();
        let lease = registry.acquire(workspace_id, 9);
        let store = open_test_store(&temp, "pending-reacquire.sqlite").await;

        assert!(
            registry.queue_close(workspace_id, 9, store).is_none(),
            "active lease should defer the close"
        );

        let reopened = registry
            .acquire_pending_close_store(workspace_id)
            .expect("deferred closes should lend the draining store");
        drop(reopened);
        drop(lease);

        tokio::time::timeout(
            Duration::from_secs(1),
            registry.wait_for_workspace_close(workspace_id),
        )
        .await
        .expect("deferred close should eventually complete");
    }
}
