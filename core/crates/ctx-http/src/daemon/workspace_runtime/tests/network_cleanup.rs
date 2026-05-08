use super::fixtures::*;
use super::*;

#[test]
fn sandbox_machine_temp_state_paths_match_expected_names() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let paths = sandbox_machine_temp_state_paths(data_root.path(), "ctx");
    let rendered: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let expected_tmp_prefix = sandbox_machine_temp_root(data_root.path())
        .join("sandbox-cli")
        .to_string_lossy()
        .to_string();
    assert!(rendered.iter().any(|p| p.starts_with(&expected_tmp_prefix)));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("sandbox-cli/gvproxy.pid")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("sandbox-cli/ctx-api.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("sandbox-cli/ctx-gvproxy.sock")));
    assert!(rendered.iter().any(|p| p.ends_with("sandbox-cli/ctx.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("home/.sandbox-cli/ctx-api.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("home/.sandbox-cli/ctx-gvproxy.sock")));
}

#[tokio::test]
async fn sandbox_machine_singleflight_lock_reuses_lock_for_same_machine() {
    let first = sandbox_machine_singleflight_lock("ctx-machine-a");
    let second = sandbox_machine_singleflight_lock("ctx-machine-a");
    assert!(Arc::ptr_eq(&first, &second));

    let guard = first.lock().await;
    assert!(second.try_lock().is_err());
    drop(guard);
    assert!(second.try_lock().is_ok());
}

#[tokio::test]
async fn sandbox_machine_singleflight_lock_isolated_by_machine_name() {
    let first = sandbox_machine_singleflight_lock("ctx-machine-b");
    let second = sandbox_machine_singleflight_lock("ctx-machine-c");
    assert!(!Arc::ptr_eq(&first, &second));

    let _guard = first.lock().await;
    assert!(second.try_lock().is_ok());
}

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

#[cfg(unix)]
#[test]
fn kill_ctx_managed_sandbox_helper_processes_reports_only_successful_kills() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
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
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        sandbox_machine_temp_root(temp.path())
            .join("sandbox-cli")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );
    let vfkit = format!(
        "/opt/homebrew/bin/vfkit --device virtio-blk,path={} --device virtio-net,unixSocketPath={}",
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
            .join(format!("{machine_name}-gvproxy.sock"))
            .display(),
    );
    let escaped_gvproxy = literal_pkill_pattern(&gvproxy);
    let escaped_vfkit = literal_pkill_pattern(&vfkit);

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n12484 {vfkit}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf '12484 {vfkit}\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
                vfkit = vfkit,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
            &kill_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nif [ \"$4\" = '{vfkit}' ]; then\n  exit 1\nfi\nexit 0\n",
                log = pkill_log_path.display(),
                vfkit = escaped_vfkit,
            ),
        )
        .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_sandbox_helper_processes(temp.path(), &machine_name);
    assert_eq!(outcome.killed, vec![6622]);
    assert_eq!(outcome.failed, vec![12484]);
    assert!(outcome.skipped.is_empty());

    let kill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert!(kill_log.contains(&format!("-9 -f -x {escaped_gvproxy}")));
    assert!(kill_log.contains(&format!("-9 -f -x {escaped_vfkit}")));
}

#[cfg(unix)]
#[test]
fn kill_ctx_managed_sandbox_helper_processes_escapes_regex_metacharacters_for_pkill() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
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
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        sandbox_machine_temp_root(temp.path())
            .join("sandbox-cli")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
        &kill_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$4\" >> \"{log}\"\nexit 0\n",
            log = pkill_log_path.display(),
        ),
    )
    .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_sandbox_helper_processes(temp.path(), &machine_name);
    assert_eq!(outcome.killed, vec![6622]);
    assert!(outcome.failed.is_empty());
    assert!(outcome.skipped.is_empty());

    let pkill_pattern = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert_eq!(pkill_pattern.trim(), literal_pkill_pattern(&gvproxy));
}

#[cfg(unix)]
#[test]
fn kill_ctx_managed_sandbox_helper_processes_skips_reused_pid_after_command_scoped_kill() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
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
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        sandbox_machine_temp_root(temp.path())
            .join("sandbox-cli")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf ' 6622 /usr/bin/python3 /tmp/not-ctx-helper.py\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
        &kill_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nexit 1\n",
            log = pkill_log_path.display(),
        ),
    )
    .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_sandbox_helper_processes(temp.path(), &machine_name);
    assert!(outcome.killed.is_empty());
    assert!(outcome.failed.is_empty());
    assert_eq!(outcome.skipped, vec![6622]);
    let pkill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert!(pkill_log.contains(&format!("-9 -f -x {}", literal_pkill_pattern(&gvproxy))));
}
