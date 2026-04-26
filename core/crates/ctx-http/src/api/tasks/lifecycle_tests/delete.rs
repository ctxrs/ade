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

    let status = delete_task(
        State(Arc::clone(&state)),
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

#[tokio::test]
async fn delete_task_cleans_up_archived_subagent_worktree() {
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
    let (parent_worktree, parent_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    let (child_worktree, child_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    for worktree in [&parent_worktree, &child_worktree] {
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
            .expect("upsert worktree index");
    }
    store
        .set_task_primary_worktree(task.id, parent_worktree.id)
        .await
        .expect("set primary worktree");
    let parent_session = store
        .create_session(
            task.id,
            workspace.id,
            parent_worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "parent".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create parent session");
    let child_session = store
        .create_session(
            task.id,
            workspace.id,
            child_worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "child".to_string(),
            Some(parent_session.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .expect("create child session");
    for session in [&parent_session, &child_session] {
        state
            .global_store()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await
            .expect("upsert session index");
    }
    assert!(store
        .archive_subagent_session(parent_session.id, child_session.id)
        .await
        .expect("archive child session"));

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(
        store
            .get_worktree(parent_worktree.id)
            .await
            .expect("load parent worktree")
            .is_none(),
        "delete should remove the parent worktree row"
    );
    assert!(
        store
            .get_worktree(child_worktree.id)
            .await
            .expect("load child worktree")
            .is_none(),
        "delete should remove the archived child worktree row"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(parent_worktree.id)
            .await
            .expect("load parent worktree index")
            .is_none(),
        "delete should remove the parent worktree index"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(child_worktree.id)
            .await
            .expect("load child worktree index")
            .is_none(),
        "delete should remove the archived child worktree index"
    );
    assert!(
        tokio::fs::metadata(&parent_root).await.is_err(),
        "delete should remove the parent managed worktree root"
    );
    assert!(
        tokio::fs::metadata(&child_root).await.is_err(),
        "delete should remove the archived child managed worktree root"
    );
    assert!(
        !branch_exists(
            &repo_root,
            parent_worktree
                .git_branch
                .as_deref()
                .expect("parent branch name"),
        )
        .await
        .expect("check parent branch"),
        "delete should remove the parent branch"
    );
    assert!(
        !branch_exists(
            &repo_root,
            child_worktree
                .git_branch
                .as_deref()
                .expect("child branch name"),
        )
        .await
        .expect("check child branch"),
        "delete should remove the archived child branch"
    );
}
