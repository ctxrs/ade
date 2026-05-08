use super::fixtures::*;
use super::*;

#[test]
fn missing_machine_error_detection_matches_expected_shapes() {
    assert!(looks_like_missing_machine_error(
        "error: no machine with this name exists"
    ));
    assert!(looks_like_missing_machine_error(
        "Error: machine ctx not found"
    ));
    assert!(!looks_like_missing_machine_error(
        "error: machine already running"
    ));
}

#[test]
fn recoverable_machine_start_error_detection_matches_expected_shapes() {
    assert!(looks_like_recoverable_machine_start_error(
        "error: machine is already starting"
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "Error: unable to start \"ctx\": already running\nStarting machine \"ctx\""
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "error: resource busy while acquiring lock"
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "error: operation timed out while waiting for vm startup"
    ));
    assert!(looks_like_recoverable_machine_start_error(
            "time=\"2026-03-05T00:23:28-06:00\" level=warning msg=\"detected port conflict on machine ssh port [49401], reassigning\"\nError: vfkit exited unexpectedly with exit code 1"
        ));
    assert!(looks_like_recoverable_machine_start_error(
        "Error: unable to connect to \"gvproxy\" socket at \"/tmp/sandbox-cli.sock\""
    ));
    assert!(!looks_like_recoverable_machine_start_error(
        "error: unknown vm provider configuration"
    ));
}

#[test]
fn running_but_unreachable_machine_start_error_detection_matches_expected_shapes() {
    assert!(looks_like_running_but_unreachable_machine_start_error(
        "Error: unable to start \"ctx\": already running"
    ));
    assert!(looks_like_running_but_unreachable_machine_start_error(
        "Error: unable to connect to \"gvproxy\" socket at \"/tmp/sandbox-cli.sock\""
    ));
    assert!(!looks_like_running_but_unreachable_machine_start_error(
        "error: resource busy while acquiring lock"
    ));
    assert!(!looks_like_running_but_unreachable_machine_start_error(
        "error: operation timed out while waiting for vm startup"
    ));
}

#[test]
fn collect_ctx_managed_sandbox_helper_pids_matches_only_ctx_scoped_helpers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join("macos")
        .join("aarch64")
        .join("sandbox-cli-5.8.0")
        .join("usr")
        .join("libexec")
        .join("sandbox-cli");
    let matches = collect_ctx_managed_sandbox_helper_pids(
        vec![
            (
                42,
                vec![
                    helper_dir.join("gvproxy").to_string_lossy().into_owned(),
                    machine_name.clone(),
                    sandbox_machine_temp_root(temp.path())
                        .join("sandbox-cli")
                        .join(format!("{machine_name}-api.sock"))
                        .to_string_lossy()
                        .into_owned(),
                ],
            ),
            (
                77,
                vec![
                    "/opt/homebrew/bin/vfkit".to_string(),
                    temp.path()
                        .join("sandbox-cli")
                        .join("xdg")
                        .join("data")
                        .join("containers")
                        .join("sandbox-cli")
                        .join("machine")
                        .join("applehv")
                        .join(format!("{machine_name}-arm64.raw"))
                        .to_string_lossy()
                        .into_owned(),
                    machine_name.clone(),
                ],
            ),
            (
                88,
                vec![
                    "/opt/homebrew/libexec/sandbox-cli/gvproxy".to_string(),
                    "/tmp/sandbox-cli/sandbox-machine-default-api.sock".to_string(),
                    "sandbox-machine-default".to_string(),
                ],
            ),
        ],
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![42, 77]);
}

#[test]
fn ctx_managed_sandbox_helper_process_detection_matches_expected_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let matching_gvproxy = vec![
        temp.path()
            .join("managed")
            .join("runtimes")
            .join("sandbox-cli")
            .join("macos")
            .join("aarch64")
            .join("sandbox-cli-5.8.0")
            .join("usr")
            .join("libexec")
            .join("sandbox-cli")
            .join("gvproxy")
            .to_string_lossy()
            .into_owned(),
        sandbox_machine_temp_root(temp.path())
            .join("sandbox-cli")
            .join(format!("{machine_name}-api.sock"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_sandbox_helper_process_command(
        &matching_gvproxy,
        temp.path(),
        &machine_name
    ));

    let matching_vfkit = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine")
            .join("applehv")
            .join(format!("{machine_name}-arm64.raw"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_sandbox_helper_process_command(
        &matching_vfkit,
        temp.path(),
        &machine_name
    ));

    let wrong_machine = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine")
            .join("applehv")
            .join("ctx-someone-else-arm64.raw")
            .to_string_lossy()
            .into_owned(),
    ];
    assert!(!is_ctx_managed_sandbox_helper_process_command(
        &wrong_machine,
        temp.path(),
        &machine_name
    ));

    let host_helper = vec![
        String::from("/opt/homebrew/libexec/sandbox-cli/gvproxy"),
        String::from("/tmp/sandbox-cli/sandbox-machine-default-api.sock"),
        String::from("sandbox-machine-default"),
    ];
    assert!(!is_ctx_managed_sandbox_helper_process_command(
        &host_helper,
        temp.path(),
        &machine_name
    ));
}

#[test]
fn collect_ctx_managed_sandbox_helper_pids_from_ps_output_matches_real_macos_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join("macos")
        .join("aarch64")
        .join("sandbox-cli-5.8.0")
        .join("usr")
        .join("libexec")
        .join("sandbox-cli");
    let gvproxy_line = format!(
            " 6622 {} -mtu 1500 -listen-vfkit unixgram://{} -forward-sock {} -forward-identity {} -pid-file {}/gvproxy.pid",
            helper_dir.join("gvproxy").display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            temp.path()
                .join("sandbox-cli")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("sandbox-cli")
                .join("machine")
                .join("machine")
                .display(),
            sandbox_machine_temp_root(temp.path()).join("sandbox-cli").display(),
        );
    let vfkit_line = format!(
            "12484 /Users/example-user/Library/Application Support/vfkit --device virtio-blk,path={} --device virtio-vsock,port=1025,socketURL={} --device virtio-net,unixSocketPath={}",
            temp.path()
                .join("sandbox-cli")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("sandbox-cli")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}.sock"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
        );
    let host_line = String::from(
            "88 /opt/homebrew/libexec/sandbox-cli/gvproxy -forward-sock /tmp/sandbox-cli/sandbox-machine-default-api.sock sandbox-machine-default",
        );
    let ps_output = format!("{gvproxy_line}\n{vfkit_line}\n{host_line}\n");

    let matches = collect_ctx_managed_sandbox_helper_pids_from_ps_output(
        &ps_output,
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![6622, 12484]);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test]
async fn ensure_sandbox_machine_running_recreates_immediately_for_already_running_unreachable_machine(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let state_path = temp.path().join("sandbox-machine-ready");
    let start_count_path = temp.path().join("sandbox-machine-start-count");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
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
        .expect("already-running unreachable machine should recover");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("info"));
    assert!(log.contains("machine start "));
    assert!(log.contains("machine inspect "));
    assert!(log.contains("machine rm -f "));
    assert!(log.contains("machine init "));
    assert!(!log.contains("machine stop "));
    machine_cache_server.abort();
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test]
async fn ensure_sandbox_machine_running_fails_fast_on_unknown_start_error() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  echo 'error: unknown vm provider configuration' >&2\n  exit 125\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let err = ensure_sandbox_machine_running_with_observer(temp.path(), None)
        .await
        .expect_err("unknown start error should fail");
    let message = format!("{err:#}");
    assert!(message.contains("unknown vm provider configuration"));

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("machine start "));
    assert!(!log.contains("machine stop "));
    assert!(!log.contains("machine rm -f "));
}

#[tokio::test]
async fn initialize_sandbox_machine_uses_init_then_start_without_now() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
            log_path.display()
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

    let mut last_err = String::new();
    initialize_sandbox_machine(temp.path(), "ctx-test-machine", None, None, &mut last_err)
        .await
        .expect("initialize machine");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("machine init ctx-test-machine"));
    assert!(log.contains("machine start ctx-test-machine"));
    assert!(!log.contains("--now"));
    assert!(last_err.is_empty());
    machine_cache_server.abort();
}

#[tokio::test]
async fn initialize_sandbox_machine_terminates_stuck_init_when_machine_is_present() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exec sleep 30\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let mut last_err = String::new();
    // Keep the outer test timeout comfortably above a single inspect timeout so the test
    // validates the kill-and-continue recovery path instead of host scheduling variance.
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        initialize_sandbox_machine_with_image(
            temp.path(),
            "ctx-test-machine",
            None,
            None,
            None,
            &mut last_err,
        ),
    )
    .await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let init_result = result
        .unwrap_or_else(|_| panic!("initialize_sandbox_machine timed out; invocation log:\n{log}"));
    init_result.expect("initialize machine");

    assert!(log.contains("machine init ctx-test-machine"));
    assert!(log.contains("machine inspect ctx-test-machine"));
    assert!(log.contains("machine start ctx-test-machine"));
    assert!(!log.contains("--now"));
}
#[tokio::test]
async fn ensure_sandbox_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"ls\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"State\":\"stopped\",\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
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
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;
    let settings = ContainerExecutionSettings {
        mount_mode: ContainerMountMode::DiskIsolated,
        machine: ctx_settings_model::ContainerMachineSettings {
            memory_profile: ctx_settings_model::ContainerMachineMemoryProfile::Custom,
            custom_memory_mb: Some(12288),
            ..ctx_settings_model::ContainerMachineSettings::default()
        },
        runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        ..ContainerExecutionSettings::default()
    };
    assert_eq!(container_machine_memory_mb(&settings), 12288);

    manager
        .ensure_sandbox_machine_materialized(&settings, None)
        .await
        .expect("machine should be recreated with desired memory");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains("volume ls --format {{.Name}}"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(log.contains(&format!("machine stop {machine_name}")));
    assert!(log.contains(&format!("machine rm -f {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(log.contains("--memory 12288"));
    machine_cache_server.abort();
}
#[tokio::test]
async fn ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"ps\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"State\":\"running\",\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
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
    let _available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let settings = ContainerExecutionSettings {
        machine: ctx_settings_model::ContainerMachineSettings {
            memory_profile: ctx_settings_model::ContainerMachineMemoryProfile::Balanced,
            ..ctx_settings_model::ContainerMachineSettings::default()
        },
        mount_mode: ContainerMountMode::Legacy,
        runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_sandbox_machine_materialized(&settings, None)
        .await
        .expect("running-but-unreachable machine should defer destructive reconfiguration");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
}
#[tokio::test]
async fn ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_state_is_unknown_and_engine_unreachable(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = sandbox_machine_name(temp.path());
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"ps\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
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
    let _available = EnvGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let settings = ContainerExecutionSettings {
        machine: ctx_settings_model::ContainerMachineSettings {
            memory_profile: ctx_settings_model::ContainerMachineMemoryProfile::Balanced,
            ..ctx_settings_model::ContainerMachineSettings::default()
        },
        mount_mode: ContainerMountMode::Legacy,
        runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_sandbox_machine_materialized(&settings, None)
        .await
        .expect("unknown runtime state should defer destructive reconfiguration");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
}
