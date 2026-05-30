use super::collect::collect_turns_by_statuses_parts;
use crate::daemon::scheduler::{
    reconcile_turn_terminal_state_with_host, DaemonTerminalStateReconcileHost,
};
use anyhow::Result;
use ctx_core::models::SessionTurnStatus;
use ctx_store::{Store, StoreManager};

#[derive(Clone)]
pub(in crate::daemon) struct StartupTurnReconcileHost {
    global_store: Store,
    stores: StoreManager,
    terminal_state: DaemonTerminalStateReconcileHost,
}

impl StartupTurnReconcileHost {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        terminal_state: DaemonTerminalStateReconcileHost,
    ) -> Self {
        Self {
            global_store,
            stores,
            terminal_state,
        }
    }

    pub(in crate::daemon) async fn reconcile_running_turns(&self) -> Result<()> {
        self.reconcile_running_turns_with_reason("daemon_restart")
            .await
    }

    pub(in crate::daemon) async fn reconcile_running_turns_with_reason(
        &self,
        fallback_reason: &str,
    ) -> Result<()> {
        let (_, running_turns) = collect_turns_by_statuses_parts(
            &self.global_store,
            &self.stores,
            &[SessionTurnStatus::Starting, SessionTurnStatus::Running],
        )
        .await?;

        for (_, turn) in running_turns {
            if let Err(err) = reconcile_turn_terminal_state_with_host(
                &self.terminal_state,
                turn.session_id,
                turn.run_id,
                turn.turn_id,
                fallback_reason,
            )
            .await
            {
                tracing::warn!(
                    session_id = %turn.session_id.0,
                    turn_id = %turn.turn_id.0,
                    err = %err,
                    "failed to reconcile running turn after daemon restart"
                );
            }
        }

        Ok(())
    }
}
