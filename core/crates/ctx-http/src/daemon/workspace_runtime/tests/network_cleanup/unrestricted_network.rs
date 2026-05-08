use super::*;

#[cfg(unix)]
#[tokio::test]
async fn unrestricted_network_transition_surfaces_teardown_failures() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_log_path = temp.path().join("cleanup-helpers.log");
    let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
    std::fs::write(&pid_file_path, b"\n").expect("write fake proxy pid file");
    let _pid_file_guard = EnvGuard::set(
        "CTX_EGRESS_PROXY_PID_FILE",
        &pid_file_path.to_string_lossy(),
    );

    let rm_path = fakebin.join("rm");
    std::fs::write(
            &rm_path,
            format!(
                "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\necho 'failed to remove proxy pid file' >&2\nexit 23\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake rm");
    std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake rm");

    let iptables_path = fakebin.join("iptables");
    std::fs::write(
            &iptables_path,
            format!(
                "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nif [ \"$1\" = \"-P\" ] && [ \"$2\" = \"OUTPUT\" ] && [ \"$3\" = \"ACCEPT\" ]; then\n  echo 'failed to reset output policy' >&2\n  exit 42\nfi\nexit 0\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake iptables");
    std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake iptables");

    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let settings = ContainerExecutionSettings {
        network_mode: ContainerNetworkMode::All,
        runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        ..Default::default()
    };
    let err = apply_container_network_policy(
        temp.path(),
        &SandboxCommandMode::NativeContainer,
        WorkspaceId::new(),
        "ctx-harness-test",
        &settings,
        "127.0.0.1",
        4399,
    )
    .await
    .expect_err("teardown failure should be explicit");

    let message = format!("{err:#}");
    assert!(
        message.contains("failed to tear down restricted container network policy"),
        "unexpected teardown error: {message}"
    );
    assert!(
        message.contains("stop transparent proxy"),
        "unexpected teardown error: {message}"
    );
    assert!(
        message.contains("clear egress guard"),
        "unexpected teardown error: {message}"
    );
    assert!(
        message.contains("failed to reset output policy"),
        "unexpected teardown error: {message}"
    );

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert_eq!(
        log.lines().filter(|line| line.starts_with("exec ")).count(),
        2,
        "expected both teardown steps to run before surfacing the failure"
    );

    let helper_log =
        std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
    assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
    assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
    assert!(helper_log.contains("iptables -F OUTPUT"));
    assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
}

#[cfg(unix)]
#[tokio::test]
async fn unrestricted_network_transition_ignores_stale_proxy_pid_file() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("sandbox-cli-invocations.log");
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_log_path = temp.path().join("cleanup-helpers.log");
    let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
    std::fs::write(&pid_file_path, b"999999\n").expect("write stale proxy pid file");
    let _pid_file_guard = EnvGuard::set(
        "CTX_EGRESS_PROXY_PID_FILE",
        &pid_file_path.to_string_lossy(),
    );

    let rm_path = fakebin.join("rm");
    std::fs::write(
        &rm_path,
        format!(
            "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\nexec /bin/rm \"$@\"\n",
            log = helper_log_path.display(),
        ),
    )
    .expect("write fake rm");
    std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake rm");

    let iptables_path = fakebin.join("iptables");
    std::fs::write(
        &iptables_path,
        format!(
            "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nexit 0\n",
            log = helper_log_path.display(),
        ),
    )
    .expect("write fake iptables");
    std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake iptables");

    let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
    std::fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _guard = EnvGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let settings = ContainerExecutionSettings {
        network_mode: ContainerNetworkMode::All,
        runtime: ctx_settings_model::ContainerRuntimeKind::NativeContainer,
        ..Default::default()
    };
    let applied = apply_container_network_policy(
        temp.path(),
        &SandboxCommandMode::NativeContainer,
        WorkspaceId::new(),
        "ctx-harness-test",
        &settings,
        "127.0.0.1",
        4399,
    )
    .await
    .expect("stale proxy pid should be ignored during unrestricted teardown");

    assert!(!applied.egress_guard);
    assert!(
        !pid_file_path.exists(),
        "stale proxy pid file should be removed during teardown"
    );

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert_eq!(
        log.lines().filter(|line| line.starts_with("exec ")).count(),
        2,
        "expected both unrestricted teardown steps to run"
    );

    let helper_log =
        std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
    assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
    assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
    assert!(helper_log.contains("iptables -F OUTPUT"));
    assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
}
