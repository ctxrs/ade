use super::super::*;

#[tokio::test]
async fn unarchive_task_does_not_recreate_archived_subagent_worktrees() {
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
            ExecutionEnvironment::Host,
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
            ExecutionEnvironment::Host,
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
    store
        .archive_subagent_session(parent_session.id, child_session.id)
        .await
        .expect("archive child session");

    git(
        &[
            "worktree",
            "remove",
            "--force",
            child_root.to_string_lossy().as_ref(),
        ],
        &repo_root,
    );
    git(
        &[
            "branch",
            "-D",
            child_worktree.git_branch.as_deref().expect("child branch"),
        ],
        &repo_root,
    );
    assert!(
        tokio::fs::metadata(&child_root).await.is_err(),
        "test setup should remove the archived child managed worktree root"
    );

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");
    let Json(unarchived) = unarchive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("unarchive task");

    assert!(unarchived.archived_at.is_none());
    assert!(
        tokio::fs::metadata(&parent_root).await.is_ok(),
        "unarchive should recreate the active parent managed worktree root"
    );
    assert!(
        tokio::fs::metadata(&child_root).await.is_err(),
        "unarchive should not recreate archived subagent managed worktrees"
    );
    assert!(
        branch_exists(
            &repo_root,
            parent_worktree
                .git_branch
                .as_deref()
                .expect("parent branch name"),
        )
        .await
        .expect("check parent branch"),
        "unarchive should restore the active parent branch"
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
        "unarchive should not restore archived subagent branches"
    );
}
