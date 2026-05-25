use std::sync::Arc;

use ctx_core::models::SessionEvent;

use crate::daemon::state::{DaemonState, ProtectedWorkspaceStoreLookup, SessionStoreLookup};

pub async fn publish_event(state: &Arc<DaemonState>, event: SessionEvent) {
    let workspace_stores = ProtectedWorkspaceStoreLookup::new(
        state.core.stores.clone(),
        Arc::clone(&state.sessions),
        Arc::clone(&state.transport.merge_queue),
    );
    let session_stores =
        SessionStoreLookup::new(state.global_store().clone(), workspace_stores.clone());
    let task_publication = Arc::new(
        crate::daemon::task_session_effects::TaskPublicationHost::new(
            workspace_stores,
            Arc::clone(&state.workspaces.workspace_active_snapshot),
        ),
    );
    crate::daemon::task_session_effects::SessionPublicationEffects::new(
        Arc::clone(&state.sessions),
        session_stores,
        task_publication,
    )
    .publish_event(event)
    .await;
}
