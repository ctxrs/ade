use super::*;

#[tokio::test]
async fn delete_task_preserves_worktree_for_archived_sibling_session_reference() {
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
    let active_task = store
        .create_task(workspace.id, "active".to_string(), None)
        .await
        .expect("create active task");
    let archived_task = store
        .create_task(workspace.id, "archived".to_string(), None)
        .await
        .expect("create archived task");
    state
        .global_store()
        .upsert_workspace_task_index(active_task.id, workspace.id)
        .await
        .expect("upsert active task index");
    state
        .global_store()
        .upsert_workspace_task_index(archived_task.id, workspace.id)
        .await
        .expect("upsert archived task index");
    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        active_task.id,
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
        .set_task_primary_worktree(active_task.id, worktree.id)
        .await
        .expect("set active primary worktree");
    store
        .create_session(
            archived_task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "archived".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create archived task session");
    store
        .archive_task(archived_task.id)
        .await
        .expect("archive sibling task");

    let (sessions, providers, workspaces, transport) = task_api_states(&state);
    let status = delete_task(
        sessions,
        providers,
        workspaces,
        transport,
        Path(active_task.id.0.to_string()),
    )
    .await
    .expect("delete active task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_task(archived_task.id)
            .await
            .expect("load archived sibling")
            .expect("archived sibling exists")
            .archived_at
            .is_some(),
        "archived sibling should remain archived after deleting the active task"
    );
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "delete should preserve the worktree row while an archived sibling still references it"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_some(),
        "delete should preserve the worktree index while an archived sibling still references it"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "delete should preserve the managed worktree root for archived sibling history"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "delete should preserve the branch while an archived sibling still references the worktree"
    );
}
