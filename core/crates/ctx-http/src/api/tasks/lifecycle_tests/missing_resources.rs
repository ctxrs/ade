use super::*;
use std::sync::Arc;

#[tokio::test]
async fn task_mutations_return_not_found_for_unknown_task() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path()).await;
    let missing_task_id = TaskId::new();
    assert_task_mutations_return_not_found(&state, missing_task_id).await;
}

#[tokio::test]
async fn task_mutations_return_not_found_for_stale_task_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path()).await;
    let stale_task_id = TaskId::new();
    state
        .global_store()
        .upsert_workspace_task_index(stale_task_id, WorkspaceId::new())
        .await
        .expect("seed stale task index");

    assert_task_mutations_return_not_found(&state, stale_task_id).await;
}

async fn assert_task_mutations_return_not_found(state: &Arc<DaemonState>, missing_task_id: TaskId) {
    let tasks = task_api_task_state(state);

    let task_sessions_status =
        list_task_sessions(tasks.clone(), Path(missing_task_id.0.to_string()))
            .await
            .expect_err("missing task sessions should fail");
    assert_eq!(task_sessions_status, StatusCode::NOT_FOUND);

    let read_status = mark_task_read(tasks.clone(), Path(missing_task_id.0.to_string()))
        .await
        .expect_err("missing task read should fail");
    assert_eq!(read_status, StatusCode::NOT_FOUND);

    let unread_status = mark_task_unread(tasks.clone(), Path(missing_task_id.0.to_string()))
        .await
        .expect_err("missing task unread should fail");
    assert_eq!(unread_status, StatusCode::NOT_FOUND);

    let title_update = crate::daemon::DaemonHandle::new(Arc::clone(&state))
        .tasks()
        .update_task_title(missing_task_id, "renamed".to_string())
        .await
        .expect("missing task title update should not be an internal error");
    assert!(title_update.is_none());

    let archive_status = archive_task(tasks.clone(), Path(missing_task_id.0.to_string()))
        .await
        .expect_err("missing task archive should fail");
    assert_eq!(archive_status, StatusCode::NOT_FOUND);

    let unarchive_status = unarchive_task(tasks, Path(missing_task_id.0.to_string()))
        .await
        .expect_err("missing task unarchive should fail");
    assert_eq!(unarchive_status, StatusCode::NOT_FOUND);
}
