use super::avf_fixtures::*;
use super::fixtures::*;
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn ready_runtime_sandbox_cli_short_circuits_network_cleanup_scripts_for_sh_c() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let _helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(temp.path());
    let _sandbox_cli_available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvGuard::set(
        crate::daemon::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );

    let mut cmd = sandbox_container_command(temp.path()).expect("sandbox container command");
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg("ctx-harness-network-cleanup")
        .arg("sh")
        .arg("-c")
        .arg("iptables -t nat -F OUTPUT");
    let output = command_output_with_timeout(cmd, Duration::from_secs(60))
        .await
        .expect("run fake network cleanup");
    assert!(
        output.status.success(),
        "fake AVF sandbox CLI should short-circuit host network cleanup scripts: {}",
        command_output_message(&output)
    );
}

#[tokio::test]
async fn sandbox_mode_errors_when_container_cli_is_unavailable() {
    let _serial = env_var_test_lock().lock().await;
    let _guard = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let tmp = tempfile::tempdir().unwrap();
    let manager = runtime_manager(&tmp).await;
    let workspace = sample_workspace(&tmp);
    let worktree = sample_worktree(&tmp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
            ..ContainerExecutionSettings::default()
        },
    };

    let err = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:9999")
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("native sandbox container runtime")
            || message.contains("sandbox container CLI unavailable")
            || message.contains("container runtime failed"),
        "unexpected error message: {message}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_in_host_mode_does_not_refresh_local_sandbox_activity() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let before_prepare = Instant::now() - Duration::from_secs(600);
    manager.set_last_activity_for_test(before_prepare);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Host,
        container: ContainerExecutionSettings::default(),
    };

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("host prepare should succeed without touching the local sandbox");
    match plan.runtime {
        HarnessRuntimeKind::Host => {}
        HarnessRuntimeKind::NativeContainer { .. } => panic!("expected host runtime"),
        HarnessRuntimeKind::SharedVmContainer => panic!("expected host runtime"),
    }
    assert!(
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .is_empty(),
        "host prepare should not invoke sandbox CLI"
    );
    let idle_for = manager.runtime_idle_for();
    assert!(
        idle_for >= Duration::from_secs(540),
        "host prepare should not refresh local sandbox activity; idle_for={idle_for:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_reuses_running_workspace_container_without_front_loading_image_readiness() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let container_name = workspace_container_name(workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
            ..Default::default()
        },
    };

    let sandbox_cli_path =
        write_running_container_sandbox_cli_shim(temp.path(), &log_path, &container_name);
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("running workspace container should be reused without image checks");

    match plan.runtime {
        HarnessRuntimeKind::NativeContainer { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
        HarnessRuntimeKind::SharedVmContainer => panic!("expected sandbox container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected running container existence check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected running-container inspect in log:\n{log}"
    );
    assert!(
        !log.contains("image inspect"),
        "running-container reuse should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "running-container reuse should not recreate the container:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_starts_cached_workspace_container_when_sandbox_cli_reports_it_stopped() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let container_name = workspace_container_name(workspace.id);
    let volume_name = format!("ctx-ws-{}", workspace.id.0);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            allowlist: Vec::new(),
            runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
            ..Default::default()
        },
    };
    let mount_plan = build_mounts(
        temp.path(),
        &workspace,
        Some(&worktree),
        &settings.container,
    );

    manager
        .put_cached_workspace_container_for_test(
            workspace.id,
            WorkspaceContainer {
                name: container_name.clone(),
                mount_mode: settings.container.mount_mode.clone(),
                network_mode: settings.container.network_mode.clone(),
                allowlist: settings.container.allowlist.clone(),
                external_mounts: mount_plan.external_mounts,
                egress_guard: false,
            },
        )
        .await;

    std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'false\\n'\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"{volume}\",\"Destination\":\"{workspace_root}\"}}]}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
                volume = volume_name,
                workspace_root = CTX_CONTAINER_WORKSPACE_ROOT,
            ),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("stopped cached workspace container should be restarted");

    match plan.runtime {
        HarnessRuntimeKind::NativeContainer { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
        HarnessRuntimeKind::SharedVmContainer => panic!("expected sandbox container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected container existence check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected stopped-container inspect in log:\n{log}"
    );
    assert!(
        log.contains(&format!("start {container_name}")),
        "expected stopped cached container to be started:\n{log}"
    );
    assert!(
        !log.contains("image inspect"),
        "starting a stopped cached container should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "starting a stopped cached container should not recreate the container:\n{log}"
    );
}
