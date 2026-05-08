use super::*;

#[tokio::test]
async fn archive_task_reclaims_managed_worktree_but_preserves_rematerialization_state() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
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

    let host_materialization_root = temp.path().join("host-shadow");
    std::fs::create_dir_all(&host_materialization_root).expect("create host shadow root");
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::SharedVmContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: ctx_sandbox_contract::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: None,
            container_name: Some(ctx_workspace_container::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: Some(
                host_materialization_root.to_string_lossy().to_string(),
            ),
            created_at: Utc::now(),
        })
        .await
        .expect("insert sandbox binding");

    let log_path = temp.path().join("sandbox-cli.log");
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &ctx_workspace_container::workspace_container_name(workspace.id),
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");
    let archived_task = store
        .get_task(task.id)
        .await
        .expect("load archived task")
        .expect("archived task exists");
    assert!(
        archived_task.archived_at.is_some(),
        "task should be archived after archive_task"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "archive should reclaim the canonical managed worktree root"
    );
    assert!(
        !branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "archive should reclaim the task worktree branch"
    );
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "archive should preserve the worktree row"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding")
            .is_some(),
        "archive should preserve the sandbox binding row for rematerialization"
    );
    assert!(
        tokio::fs::metadata(&host_materialization_root)
            .await
            .is_err(),
        "archive should remove the sandbox host materialization root"
    );
}

#[tokio::test]
async fn archive_task_reports_cleanup_failure_when_branch_reclaim_fails() {
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
    let branch = worktree
        .git_branch
        .as_deref()
        .expect("branch name")
        .to_string();
    let _branch_lock = create_branch_lock(&repo_root, &branch);

    let Json(response) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");

    assert!(
        response.cleanup_failed,
        "archive should report cleanup failure when branch reclaim fails"
    );
    assert!(
        response.task.archived_at.is_some(),
        "task should still be archived after reporting cleanup failure"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "archive should still reclaim the managed worktree root"
    );
    assert!(
        branch_exists(&repo_root, &branch)
            .await
            .expect("check branch"),
        "archive should report failure if the ctx branch could not be reclaimed"
    );
}
