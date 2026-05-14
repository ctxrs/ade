use super::*;

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

    let tasks = task_api_task_state(&state);
    let status = delete_task(tasks, Path(task.id.0.to_string()))
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
