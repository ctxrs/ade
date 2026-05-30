use super::*;

#[tokio::test]
async fn workspace_drain_ownership_allows_only_one_runner() {
    let (_data_dir, state) = setup_state().await;
    let workspace_id = WorkspaceId::new();
    let host = merge_queue_host(state.as_ref());

    assert!(ctx_merge_queue::begin_workspace_drain(host.as_ref(), workspace_id).await);
    assert!(!ctx_merge_queue::begin_workspace_drain(host.as_ref(), workspace_id).await);
    ctx_merge_queue::finish_workspace_drain(host.as_ref(), workspace_id).await;
    assert!(ctx_merge_queue::begin_workspace_drain(host.as_ref(), workspace_id).await);
}
