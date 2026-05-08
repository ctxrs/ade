use super::super::*;

#[tokio::test]
async fn unarchive_task_recreates_managed_root_and_keeps_binding_snapshot_runtime() {
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

    let persisted_snapshot = ctx_settings_model::ExecutionSettings {
        mode: ctx_settings_model::ExecutionMode::Sandbox,
        container: ctx_settings_model::ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
            network_mode: ctx_settings_model::ContainerNetworkMode::Allowlist,
            allowlist: vec!["github.com".to_string()],
            image: Some("registry.example/sandbox:v1".to_string()),
            ..ctx_settings_model::ContainerExecutionSettings::default()
        },
    };
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
                serde_json::to_string(&persisted_snapshot).expect("serialize binding snapshot"),
            ),
            container_name: Some(ctx_workspace_container::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: None,
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

    save_test_execution_settings(
        &state,
        ctx_settings_model::ExecutionSettings {
            mode: ctx_settings_model::ExecutionMode::Sandbox,
            container: ctx_settings_model::ContainerExecutionSettings {
                runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
                network_mode: ctx_settings_model::ContainerNetworkMode::All,
                allowlist: Vec::new(),
                image: Some("registry.example/sandbox:v2".to_string()),
                ..ctx_settings_model::ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    let Json(unarchived_task) =
        unarchive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
            .await
            .expect("unarchive task");

    assert!(
        unarchived_task.archived_at.is_none(),
        "task should no longer be archived after unarchive_task"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "unarchive should recreate the canonical managed worktree root"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "unarchive should keep the existing managed worktree branch attached"
    );

    let current_effective =
        crate::daemon::execution_effective::effective_execution_settings(&state, workspace.id)
            .await
            .expect("load current effective settings");
    assert_eq!(
        current_effective.container.runtime,
        ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        "workspace defaults should now point at the new runtime"
    );

    let binding = store
        .get_sandbox_binding(worktree.id)
        .await
        .expect("load rematerialized binding")
        .expect("binding should remain present after unarchive");
    assert_eq!(binding.substrate, SandboxSubstrate::NativeContainer);
    assert_eq!(
        binding.sandbox_instance_id,
        ctx_core::models::sandbox_instance_id_for_workspace(workspace.id)
    );
    let parsed = crate::api::tasks::sandbox_execution_settings_from_binding(&binding)
        .expect("parse rematerialized binding snapshot");
    assert_eq!(
        parsed.container.runtime,
        ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        "rematerialized binding must preserve the original runtime snapshot"
    );
    assert_eq!(
        parsed.container.network_mode,
        ctx_settings_model::ContainerNetworkMode::Allowlist
    );
    assert_eq!(parsed.container.allowlist, vec!["github.com".to_string()]);
    assert_eq!(
        parsed.container.image,
        Some("registry.example/sandbox:v1".to_string())
    );
}
