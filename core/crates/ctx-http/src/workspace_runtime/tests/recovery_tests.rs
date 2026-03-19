use super::*;

async fn save_test_execution_settings(data_root: &Path, execution: ExecutionSettings) {
    let db_dir = data_root.join("db");
    std::fs::create_dir_all(&db_dir).expect("create db dir");
    let db_path = db_dir.join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .expect("open settings db");
    crate::settings::save_settings(
        &store,
        &crate::settings::Settings {
            execution: Some(execution),
            ..crate::settings::Settings::default()
        },
    )
    .await
    .expect("save settings");
    store.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_starts_existing_workspace_container_when_not_cached() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let container_name = workspace_container_name(workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            allowlist: Vec::new(),
            ..Default::default()
        },
    };

    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"exists\" ]; then\n  echo 'unexpected image check' >&2\n  exit 125\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'false\\n'\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("existing stopped workspace container should be started");

    match plan.runtime {
        HarnessRuntimeKind::Container { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
    assert!(
        log.contains(&format!("container exists {container_name}")),
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
        !log.contains("image exists"),
        "starting a stopped container should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "starting a stopped container should not recreate the container:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stop_container_returns_false_when_missing() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let container_name = workspace_container_name(workspace.id);

    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let stopped = manager
        .stop_container(workspace.id)
        .await
        .expect("stop container");
    assert!(!stopped);

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
    assert!(
        log.contains(&format!("container exists {container_name}")),
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
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let volume_name = format!("ctx-ws-{}", workspace.id.0);

    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{volume}\" ]; then\n  exit 1\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            volume = volume_name,
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let removed = manager
        .remove_workspace_volume(workspace.id)
        .await
        .expect("remove volume");
    assert!(!removed);

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
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
async fn ensure_podman_machine_running_waits_for_readiness_on_recoverable_start_error() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let info_count_path = temp.path().join("podman-info-count");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nINFO_COUNT=\"{info_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  count=0\n  if [ -f \"$INFO_COUNT\" ]; then\n    count=$(cat \"$INFO_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$INFO_COUNT\"\n  if [ \"$count\" -ge 2 ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  echo 'error: operation timed out while waiting for vm startup' >&2\n  exit 125\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            info_count = info_count_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    ensure_podman_machine_running_with_observer(temp.path(), None)
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
async fn ensure_podman_machine_running_missing_machine_recovery_uses_configured_memory() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-ready");
    let init_state_path = temp.path().join("podman-initialized");
    let podman_path = temp.path().join("podman.sh");
    save_test_execution_settings(
        temp.path(),
        ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings {
                machine: crate::settings::ContainerMachineSettings {
                    memory_profile: crate::settings::ContainerMachineMemoryProfile::Custom,
                    custom_memory_mb: Some(6144),
                    ..crate::settings::ContainerMachineSettings::default()
                },
                ..ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nINIT_STATE=\"{init_state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  if [ -f \"$INIT_STATE\" ]; then\n    touch \"$STATE\"\n    exit 0\n  fi\n  echo 'error: machine does not exist' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  touch \"$INIT_STATE\"\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
            init_state = init_state_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_podman_machine_running_with_observer(temp.path(), None)
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
async fn ensure_podman_machine_running_recreate_recovery_uses_configured_memory() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-ready");
    let start_count_path = temp.path().join("podman-start-count");
    let podman_path = temp.path().join("podman.sh");
    save_test_execution_settings(
        temp.path(),
        ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings {
                machine: crate::settings::ContainerMachineSettings {
                    memory_profile: crate::settings::ContainerMachineMemoryProfile::Custom,
                    custom_memory_mb: Some(7168),
                    ..crate::settings::ContainerMachineSettings::default()
                },
                ..ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nSTART_COUNT=\"{start_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  count=0\n  if [ -f \"$START_COUNT\" ]; then\n    count=$(cat \"$START_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$START_COUNT\"\n  if [ \"$count\" -eq 1 ]; then\n    echo 'Error: unable to start \"ctx\": already running' >&2\n    exit 125\n  fi\n  touch \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nexit 0\n",
            log = log_path.display(),
            state = state_path.display(),
            start_count = start_count_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_podman_machine_running_with_observer(temp.path(), None)
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
