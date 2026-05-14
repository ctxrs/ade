use super::*;

#[tokio::test]
async fn delete_task_removes_worktree_metadata_even_when_branch_reclaim_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");

    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let _branch_lock = create_branch_lock(
        &repo_root,
        worktree.git_branch.as_deref().expect("branch name"),
    );

    let (sessions, providers, workspaces, transport) = task_api_states(&state);
    let status = delete_task(
        sessions,
        providers,
        workspaces,
        transport,
        Path(task.id.0.to_string()),
    )
    .await
    .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_none(),
        "branch reclaim failure must not keep the worktree row behind"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_none(),
        "branch reclaim failure must not keep the worktree index behind"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "managed worktree root should still be removed before branch reclaim is attempted"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "branch lock should keep the branch behind so the test exercises best-effort branch cleanup"
    );
}
