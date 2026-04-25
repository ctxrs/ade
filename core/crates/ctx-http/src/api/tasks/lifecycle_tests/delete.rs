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
async fn delete_task_cleanup_errors_preserve_worktree_row_and_index() {
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
    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(temp.path(), workspace.id, worktree_id);
    std::fs::create_dir_all(
        managed_root
            .parent()
            .expect("managed worktree parent exists"),
    )
    .expect("create managed worktree parent");
    std::fs::write(&managed_root, "not-a-directory").expect("create managed worktree file");
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.clone(),
            git_branch: Some(format!("ctx/{}/{}", task.id.0, worktree_id.0)),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit),
            vcs_ref: Some("".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "delete cleanup errors must not drop the worktree row"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_some(),
        "delete cleanup errors must not drop the worktree index"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "failed cleanup should leave the managed worktree root in place"
    );
}

#[tokio::test]
async fn delete_task_removes_unused_worktree_rows_and_indexes() {
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

    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(temp.path(), workspace.id, worktree_id);
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            managed_root.to_string_lossy().as_ref(),
            &base_commit,
        ],
        &repo_root,
    );
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.clone(),
            git_branch: Some(branch_name),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit.clone()),
            vcs_ref: Some("".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_none(),
        "unused worktree row should be removed on task delete"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_none(),
        "unused worktree index should be removed on task delete"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "managed worktree root should be removed on task delete"
    );
}

#[tokio::test]
async fn delete_task_removes_standalone_managed_worktree_when_workspace_root_is_missing() {
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
    ctx_fs::worktrees::standaloneize_worktree_git_dir(&managed_root)
        .await
        .expect("standaloneize managed worktree");
    tokio::fs::remove_dir_all(&repo_root)
        .await
        .expect("remove source workspace root");

    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_none(),
        "unused worktree row should be removed on task delete"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_none(),
        "unused worktree index should be removed on task delete"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "standalone managed worktree root should be removed on task delete"
    );
}
