use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::ExecutionEnvironment;
use ctx_store::StoreManager;

pub(super) fn session_id(value: &str) -> SessionId {
    SessionId(uuid::Uuid::parse_str(value).unwrap())
}

pub(super) async fn test_state(root: &Path) -> Arc<AppState> {
    Arc::new(AppState::new(
        root.to_path_buf(),
        StoreManager::open(root).await.unwrap(),
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ))
}

pub(super) async fn create_workspace_session(
    state: &Arc<AppState>,
    root: &Path,
) -> (WorkspaceId, SessionId) {
    let workspace = state
        .global_store()
        .create_workspace(
            format!("ws-{}", uuid::Uuid::new_v4()),
            root.join(format!("ws-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
            ctx_core::models::VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            root.join(format!("worktree-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
            "deadbeef".to_string(),
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
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    (workspace.id, session.id)
}
