use tokio::sync::watch;

use ctx_core::ids::WorktreeId;

use crate::daemon::state::{TimedEntry, WorkspaceRuntime, WorktreeBootstrapGate};

impl WorkspaceRuntime {
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
}
