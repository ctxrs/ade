use super::*;

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
