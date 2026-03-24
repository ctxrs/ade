mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::{MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, Workspace};
use ctx_http::daemon::AppState;
use serde_json::Value;

fn queued_entry(workspace_id: WorkspaceId, name: &str) -> MergeQueueEntry {
    let now = Utc::now();
    MergeQueueEntry {
        id: MergeQueueEntryId::new(),
        workspace_id,
        worktree_id: None,
        session_id: None,
        target_branch: "main".to_string(),
        message: Some(name.to_string()),
        patch_source: MergeQueuePatchSource::Generated,
        base_commit_sha: Some(format!("{name}-base")),
        head_commit_sha: Some(format!("{name}-head")),
        patch_path: format!("/tmp/{name}.patch"),
        patch_size: 1,
        status: MergeQueueEntryStatus::Queued,
        result_commit_sha: None,
        error_message: None,
        created_at: now,
        updated_at: now,
    }
}

async fn wait_for_non_queued_status(
    state: &Arc<AppState>,
    workspace: &Workspace,
    entry_id: MergeQueueEntryId,
) -> MergeQueueEntry {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let store = state.core.stores.workspace(workspace.id).await.unwrap();
            let entry = store
                .get_merge_queue_entry(entry_id)
                .await
                .unwrap()
                .unwrap();
            if entry.status != MergeQueueEntryStatus::Queued {
                break entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for merge queue entry to resume")
}

#[tokio::test]
async fn enabling_merge_queue_on_open_workspace_reschedules_queued_rows() {
    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let workspace = common::create_workspace(&app, repo.path(), "ws").await;

    ctx_http::merge_queue::spawn_merge_queue_runner(state.clone());

    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let entry = queued_entry(workspace.id, "queued-before-enable");
    store.create_merge_queue_entry(&entry).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued = state
        .core
        .stores
        .workspace(workspace.id)
        .await
        .unwrap()
        .get_merge_queue_entry(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, MergeQueueEntryStatus::Queued);

    let (set_status, set_resp): (StatusCode, Value) = common::json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/merge_queue_config", workspace.id.0),
        Some(serde_json::json!({
            "enabled": true,
            "target_branch": "main"
        })),
    )
    .await;
    assert_eq!(set_status, StatusCode::OK);
    assert_eq!(set_resp.get("ok").and_then(Value::as_bool), Some(true));

    let resumed = wait_for_non_queued_status(&state, &workspace, entry.id).await;
    assert_ne!(resumed.status, MergeQueueEntryStatus::Queued);
}
