use super::avf_fixtures::*;
use super::fixtures::*;
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn ensure_container_machine_ready_prefetches_avf_runtime_without_starting_a_global_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let settings = ContainerExecutionSettings {
        runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("AVF runtime prefetch should succeed");

    let runtime_state = super::selected_runtime_state(temp.path(), &settings)
        .await
        .expect("read AVF runtime state");
    assert_eq!(runtime_state, (true, true));

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_returns_avf_linux_vm_plan_after_workspace_vm_and_container_ready() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let (_image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should now return a container-backed AVF plan");
    match &plan.runtime {
        HarnessRuntimeKind::SharedVmContainer => {}
        HarnessRuntimeKind::Host => panic!("expected AVF Linux VM runtime"),
        HarnessRuntimeKind::NativeContainer { .. } => panic!("expected AVF Linux VM runtime"),
    }
    assert_eq!(
        plan.env_overrides
            .get(ctx_harness_runtime::CTX_HARNESS_RUNTIME_KIND_ENV)
            .map(String::as_str),
        Some("shared_vm_container")
    );
    let workspace_id = workspace.id.0.to_string();
    let worktree_id = worktree.id.0.to_string();
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_WORKSPACE_ID")
            .map(String::as_str),
        Some(workspace_id.as_str())
    );
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_WORKTREE_ID")
            .map(String::as_str),
        Some(worktree_id.as_str())
    );
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_HOST_WORKTREE_ROOT")
            .map(String::as_str),
        Some(worktree.root_path.as_str())
    );
    assert_eq!(
        plan.env_overrides.get("CTX_DAEMON_URL").map(String::as_str),
        Some("http://192.168.64.1:4399")
    );
    assert!(plan
        .env_overrides
        .contains_key(super::AVF_LINUX_HELPER_PATH_ENV));

    let state = ctx_avf_linux_runtime::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        ctx_avf_linux_runtime::AvfLinuxSharedVmLifecycleState::Running
    );

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn container_status_reports_running_avf_workspace_container() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let (_image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should start the workspace VM");

    let status = manager
        .container_status(workspace.id)
        .await
        .expect("AVF workspace status")
        .expect("AVF workspace container status should exist");
    assert_eq!(status.name, format!("ctx-harness-{}", workspace.id.0));
    assert!(status.running);
    assert!(status.known);
    assert_eq!(status.mount_mode, Some(ContainerMountMode::DiskIsolated));

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_starts_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let (_image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let workspace = sample_workspace(&temp);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_workspace_container(&workspace, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF workspace VM should be started for workspace container callers");

    let state = ctx_avf_linux_runtime::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        ctx_avf_linux_runtime::AvfLinuxSharedVmLifecycleState::Running
    );

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_for_worktree_keeps_avf_workspace_container_ready() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let (_image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_workspace_container_for_worktree(
            &workspace,
            &worktree,
            &settings,
            "http://192.168.64.1:4399",
        )
        .await
        .expect("AVF worktree ensure should keep the workspace sandbox ready");

    let status = manager
        .container_status(workspace.id)
        .await
        .expect("AVF workspace status")
        .expect("AVF workspace container state should exist");
    assert!(status.running);

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn shared_vm_container_launch_omits_slirp_network_flag() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let image_bytes = b"ctx-harness-image".to_vec();
    let (image_url, image_server) =
        spawn_static_http_server_with_suffix(image_bytes.clone(), "ctx-harness.tar").await;
    let _image_guard = bundled_assets::override_managed_ctx_harness_image_source_for_test(
        bundled_assets::ManagedArtifactSource {
            uri: image_url,
            sha256: hex::encode(Sha256::digest(&image_bytes)),
        },
    );
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should use the shared-VM container runtime");

    let log_path = temp.path().join(format!(
        "managed/vms/avf-linux/{}/{}/shared/sandbox-cli-invocations.log",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let log = std::fs::read_to_string(&log_path).expect("read AVF sandbox CLI invocation log");
    let run_line = log
        .lines()
        .find(|line| line.contains("run") && line.contains("ctx-harness-"))
        .expect("shared VM sandbox CLI run invocation");
    assert!(
        !run_line.contains("slirp4netns:allow_host_loopback=true"),
        "shared VM run should not use stale slirp networking: {run_line}"
    );
    assert!(
        run_line.contains("--add-host host.containers.internal:host-gateway"),
        "shared VM run should keep host gateway mapping: {run_line}"
    );
    assert!(
        run_line.contains("--hostname ws-container"),
        "shared VM run should use the human-readable workspace hostname: {run_line}"
    );
    for (guest_src, dst_suffix) in [
        (
            format!(
                "/mnt/ctx-host/containers/workspaces/{}/data",
                workspace.id.0
            ),
            format!("/containers/workspaces/{}/data", workspace.id.0),
        ),
        (
            "/mnt/ctx-host/providers/agent-servers".to_string(),
            "/providers/agent-servers".to_string(),
        ),
        (
            "/mnt/ctx-host/runtimes".to_string(),
            "/runtimes".to_string(),
        ),
        (
            "/mnt/ctx-host/vcs-hooks".to_string(),
            "/vcs-hooks".to_string(),
        ),
    ] {
        assert!(
            run_line.contains(&format!("src={guest_src}")),
            "shared VM run should use guest-visible mount source {guest_src}: {run_line}"
        );
        assert!(
            run_line.contains(&format!("dst={}{}", temp.path().display(), dst_suffix)),
            "shared VM run should preserve destination path {}{}: {run_line}",
            temp.path().display(),
            dst_suffix
        );
        assert!(
            !run_line.contains(&format!("src={}{}", temp.path().display(), dst_suffix)),
            "shared VM run should not leak host data-root source {}{}: {run_line}",
            temp.path().display(),
            dst_suffix
        );
    }
    assert!(
        log.lines().any(|line| {
            line.contains("exec --user 0")
                && line.contains("CTX_CONTAINER_TERMINAL_USER=ctx-user")
                && line.contains("ctx-harness-")
        }),
        "shared VM prepare should synchronize the terminal identity inside the container: {log}"
    );

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_after_runtime_ready_starts_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let image_bytes = b"ctx-harness-image".to_vec();
    let (image_url, image_server) =
        spawn_static_http_server_with_suffix(image_bytes.clone(), "ctx-harness.tar").await;
    let _image_guard = bundled_assets::override_managed_ctx_harness_image_source_for_test(
        bundled_assets::ManagedArtifactSource {
            uri: image_url,
            sha256: hex::encode(Sha256::digest(&image_bytes)),
        },
    );
    let workspace = sample_workspace(&temp);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_container_machine_ready(&settings.container, None)
        .await
        .expect("AVF runtime artifacts should prewarm");
    manager
        .ensure_workspace_container_after_runtime_ready_with_observer(
            &workspace,
            &settings,
            "http://192.168.64.1:4399",
            None,
        )
        .await
        .expect("AVF workspace VM should start from runtime-ready path");

    let state = ctx_avf_linux_runtime::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        ctx_avf_linux_runtime::AvfLinuxSharedVmLifecycleState::Running
    );
    let log_path = temp.path().join(format!(
        "managed/vms/avf-linux/{}/{}/shared/sandbox-cli-invocations.log",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let log = std::fs::read_to_string(&log_path).expect("read AVF sandbox CLI invocation log");
    assert!(
        log.contains("load -i"),
        "runtime-ready path should still load the managed harness image before run: {log}"
    );

    for server in servers {
        server.abort();
    }
    image_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn stop_container_removes_avf_workspace_container() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let (_image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            mount_mode: ContainerMountMode::DiskIsolated,
            network_mode: ContainerNetworkMode::All,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should start the workspace sandbox");

    assert!(manager
        .stop_container(workspace.id)
        .await
        .expect("stop AVF workspace container"));

    let status = manager
        .container_status(workspace.id)
        .await
        .expect("read stopped AVF workspace container status");
    assert!(
        status.as_ref().map(|value| !value.running).unwrap_or(true),
        "expected stopped or absent sandbox container status, got {status:?}"
    );

    for server in servers {
        server.abort();
    }
    image_server.abort();
}
