use super::fixtures::*;
use super::*;
use ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT;
use ctx_settings_model::ContainerNetworkMode;
use ctx_workspace_container::workspace_container_name;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
async fn save_test_execution_settings(data_root: &Path, execution: ExecutionSettings) {
    let db_dir = data_root.join("db");
    std::fs::create_dir_all(&db_dir).expect("create db dir");
    let db_path = db_dir.join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .expect("open settings db");
    ctx_settings_service::save_settings(
        &store,
        &ctx_settings_model::Settings {
            execution: Some(execution),
            ..ctx_settings_model::Settings::default()
        },
    )
    .await
    .expect("save settings");
    store.close().await;
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
async fn write_invalid_test_execution_settings(data_root: &Path, settings_json: &str) {
    let db_dir = data_root.join("db");
    std::fs::create_dir_all(&db_dir).expect("create db dir");
    let db_path = db_dir.join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .expect("open settings db");
    store
        .upsert_runtime_settings_document(1, settings_json)
        .await
        .expect("write invalid runtime settings");
    store.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_starts_existing_workspace_container_when_not_cached() {
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

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'unexpected image check' >&2\n  exit 125\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'false\\n'\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"{volume}\",\"Destination\":\"{workspace_root}\"}}]}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
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
        .expect("existing stopped workspace container should be started");

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
        "expected stopped container to be started:\n{log}"
    );
    assert!(
        !log.contains("image inspect"),
        "starting a stopped container should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "starting a stopped container should not recreate the container:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_adopts_existing_workspace_container_when_run_reports_name_collision() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let exists_state_path = temp.path().join("container-exists-count");
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

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  count=0\n  if [ -f \"$STATE\" ]; then\n    count=$(cat \"$STATE\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$STATE\"\n  if [ \"$count\" -eq 1 ]; then\n    exit 1\n  fi\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'time=\"2026-03-26T00:00:00Z\" level=fatal msg=\"name-store error\\\\nname \\\\\\\"{container}\\\\\\\" is already used by ID \\\\\\\"abc123\\\\\\\"\"\\n' >&2\n  exit 1\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"{volume}\",\"Destination\":\"{workspace_root}\"}}]}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = exists_state_path.display(),
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
        .expect("name-collision create should adopt existing container");

    match plan.runtime {
        HarnessRuntimeKind::NativeContainer { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
        HarnessRuntimeKind::SharedVmContainer => panic!("expected sandbox container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("run -d --name {container_name}")),
        "expected attempted container create in log:\n{log}"
    );
    assert_eq!(
        log.match_indices(&format!("container inspect {container_name}"))
            .count(),
        2,
        "expected initial miss plus adopt-exists recheck in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected adopted-container running check in log:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stop_container_returns_false_when_missing() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let container_name = workspace_container_name(workspace.id);

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let stopped = manager
        .stop_container(workspace.id)
        .await
        .expect("stop container");
    assert!(!stopped);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected container existence probe:\n{log}"
    );
    assert!(
        !log.contains("rm -f"),
        "missing container should not be removed:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn remove_workspace_volume_returns_false_when_missing() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let volume_name = format!("ctx-ws-{}", workspace.id.0);

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{volume}\" ]; then\n  exit 1\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            volume = volume_name,
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let removed = manager
        .remove_workspace_volume(workspace.id)
        .await
        .expect("remove volume");
    assert!(!removed);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("volume inspect {volume_name}")),
        "expected volume existence probe:\n{log}"
    );
    assert!(
        !log.contains("volume rm -f"),
        "missing volume should not be removed:\n{log}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn ensure_sandbox_machine_running_waits_for_readiness_on_recoverable_start_error() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let info_count_path = temp.path().join("sandbox-cli-info-count");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nINFO_COUNT=\"{info_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  count=0\n  if [ -f \"$INFO_COUNT\" ]; then\n    count=$(cat \"$INFO_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$INFO_COUNT\"\n  if [ \"$count\" -ge 2 ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  echo 'error: operation timed out while waiting for vm startup' >&2\n  exit 125\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            info_count = info_count_path.display(),
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect("recoverable start error should resolve once runtime becomes reachable");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("info"));
    assert!(log.contains("machine start "));
    assert!(
        !log.contains("machine stop "),
        "recoverable start error should not force stop:\n{log}"
    );
    assert!(
        !log.contains("machine rm -f "),
        "recoverable start error should not recreate:\n{log}"
    );
    assert!(
        !log.contains("machine init "),
        "recoverable start error should not init:\n{log}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn ensure_sandbox_machine_running_missing_machine_recovery_uses_configured_memory() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let state_path = temp.path().join("sandbox-machine-ready");
    let init_state_path = temp.path().join("sandbox-machine-initialized");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    save_test_execution_settings(
        temp.path(),
        ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: ContainerExecutionSettings {
                machine: ctx_settings_model::ContainerMachineSettings {
                    memory_profile: ctx_settings_model::ContainerMachineMemoryProfile::Custom,
                    custom_memory_mb: Some(6144),
                    ..ctx_settings_model::ContainerMachineSettings::default()
                },
                runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
                ..ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nINIT_STATE=\"{init_state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  if [ -f \"$INIT_STATE\" ]; then\n    touch \"$STATE\"\n    exit 0\n  fi\n  echo 'error: machine does not exist' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  touch \"$INIT_STATE\"\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
            init_state = init_state_path.display(),
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect("missing machine recovery should materialize machine with configured memory");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains(&format!("machine start {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(
        log.contains("--memory 6144"),
        "missing-machine recovery should preserve configured memory override:\n{log}"
    );
    machine_cache_server.abort();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn ensure_sandbox_machine_running_recreate_recovery_uses_configured_memory() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let state_path = temp.path().join("sandbox-machine-ready");
    let start_count_path = temp.path().join("sandbox-machine-start-count");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    save_test_execution_settings(
        temp.path(),
        ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: ContainerExecutionSettings {
                machine: ctx_settings_model::ContainerMachineSettings {
                    memory_profile: ctx_settings_model::ContainerMachineMemoryProfile::Custom,
                    custom_memory_mb: Some(7168),
                    ..ctx_settings_model::ContainerMachineSettings::default()
                },
                runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
                ..ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nSTART_COUNT=\"{start_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  count=0\n  if [ -f \"$START_COUNT\" ]; then\n    count=$(cat \"$START_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$START_COUNT\"\n  if [ \"$count\" -eq 1 ]; then\n    echo 'Error: unable to start \"ctx\": already running' >&2\n    exit 125\n  fi\n  touch \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nexit 0\n",
            log = log_path.display(),
            state = state_path.display(),
            start_count = start_count_path.display(),
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect("recreate recovery should reinitialize machine with configured memory");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains(&format!("machine start {machine_name}")));
    assert!(log.contains(&format!("machine rm -f {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(
        log.contains("--memory 7168"),
        "recreate recovery should preserve configured memory override:\n{log}"
    );
    machine_cache_server.abort();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn ensure_sandbox_machine_running_uses_info_fast_path_before_recovery_settings_load() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::create_dir_all(temp.path().join("db").join("db.sqlite"))
        .expect("create invalid settings store path");

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"info\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\ndone\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
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

    ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect("reachable runtime should not load recovery settings before the info fast path");

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("info"),
        "expected sandbox CLI info fast path to run:\n{log}"
    );
    assert!(
        !log.contains("machine start "),
        "reachable runtime should not attempt machine recovery:\n{log}"
    );
    assert!(
        !log.contains("machine init "),
        "reachable runtime should not attempt machine initialization:\n{log}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn ensure_sandbox_machine_running_falls_back_to_default_memory_when_recovery_settings_are_corrupt(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let state_path = temp.path().join("sandbox-machine-ready");
    let init_state_path = temp.path().join("sandbox-machine-initialized");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    write_invalid_test_execution_settings(temp.path(), "{").await;

    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nINIT_STATE=\"{init_state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  if [ -f \"$INIT_STATE\" ]; then\n    touch \"$STATE\"\n    exit 0\n  fi\n  echo 'error: machine does not exist' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  touch \"$INIT_STATE\"\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
            init_state = init_state_path.display(),
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect("corrupt recovery settings should fall back to default machine memory");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains(&format!("machine start {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(
        log.contains("--memory 6144"),
        "corrupt recovery settings should fall back to the default machine memory:\n{log}"
    );
    machine_cache_server.abort();
}
