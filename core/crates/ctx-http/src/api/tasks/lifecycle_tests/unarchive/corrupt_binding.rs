use super::super::*;
use ctx_core::models::SandboxProfile;

#[tokio::test]
async fn unarchive_task_fails_closed_for_corrupt_binding_snapshot() {
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
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::NativeContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: ctx_sandbox_contract::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: Some(
                serde_json::json!({
                    "mode": "host",
                    "container": {
                        "runtime": "native_container",
                        "mount_mode": "disk_isolated",
                        "network_mode": "all",
                        "allowlist": [],
                        "image": null
                    }
                })
                .to_string(),
            ),
            container_name: Some(ctx_workspace_container::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: None,
            created_at: Utc::now(),
        })
        .await
        .expect("insert corrupt sandbox binding");

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

    let (sessions, providers, workspaces, transport) = task_api_states(&state);
    let Json(_) = archive_task(
        sessions,
        providers,
        workspaces,
        transport,
        Path(task.id.0.to_string()),
    )
    .await
    .expect("archive task");

    let (sessions, providers, workspaces, transport) = task_api_states(&state);
    let status = unarchive_task(
        sessions,
        providers,
        workspaces,
        transport,
        Path(task.id.0.to_string()),
    )
    .await
    .expect_err("corrupt binding snapshot should fail closed");
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        store
            .get_task(task.id)
            .await
            .expect("load task after failed unarchive")
            .expect("task should still exist")
            .archived_at
            .is_some(),
        "failed unarchive should leave the task archived"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "failed unarchive should not destroy the canonical managed worktree root"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding after failed unarchive")
            .is_some(),
        "failed unarchive should preserve the persisted binding row for repair"
    );
}
