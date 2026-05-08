use super::*;

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
