use ctx_core::ids::SessionId;
use ctx_core::models::SessionHeadSnapshot;
use ctx_session_runtime::runtime::{SessionHeadRefreshHost, SessionHeadRefreshLoad};

use crate::daemon::state::DaemonState;

pub async fn refresh_session_head_cache(state: &DaemonState, session_id: SessionId) {
    state
        .sessions
        .refresh_session_head_cache_with_host(state, session_id)
        .await;
}

#[async_trait::async_trait]
impl SessionHeadRefreshHost for DaemonState {
    async fn load_active_snapshot_head(&self, session_id: SessionId) -> SessionHeadRefreshLoad {
        let store = match self.store_for_session(session_id).await {
            Ok(store) => store,
            Err(err) => {
                return SessionHeadRefreshLoad::Failed {
                    error: format!("{err:#}"),
                };
            }
        };
        match store.get_active_snapshot_head(session_id).await {
            Ok(Some(head)) => SessionHeadRefreshLoad::Found(Box::new(head)),
            Ok(None) => SessionHeadRefreshLoad::Missing,
            Err(err) => SessionHeadRefreshLoad::Failed {
                error: format!("{err:#}"),
            },
        }
    }

    async fn update_compact_session_head(&self, head: SessionHeadSnapshot) {
        self.workspaces
            .workspace_active_snapshot
            .update_compact_session_head(head)
            .await;
    }

    async fn remove_session_from_active_head_cache(&self, session_id: SessionId) {
        self.workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
    }
}
