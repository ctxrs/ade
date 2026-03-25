use super::update::remote_stop_daemon_cmd;
use super::*;

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn wait_for_pid_exit(pid: u32, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !pid_is_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    !pid_is_alive(pid)
}

#[test]
fn remote_ctx_bin_values_validate() {
    let valid = validate_remote_ctx_bin("/opt/ctx/bin/ctx").expect("absolute path is valid");
    assert_eq!(valid, "/opt/ctx/bin/ctx");
    let valid_home = validate_remote_ctx_bin("~/.ctx/bin/ctx").expect("~/ path is valid");
    assert_eq!(valid_home, "~/.ctx/bin/ctx");
    let err_empty = validate_remote_ctx_bin(" ").expect_err("empty path must fail");
    assert!(err_empty.to_string().contains("remote_ctx_bin is required"));
    let err_rel = validate_remote_ctx_bin("ctx").expect_err("relative path must fail");
    assert!(err_rel
        .to_string()
        .contains("must be an absolute path or ~/ path"));
}

#[test]
fn remote_ctx_bin_parent_dir_handles_home_and_absolute_paths() {
    assert_eq!(
        remote_ctx_bin_parent_dir("~/.ctx/bin/ctx").expect("home-based path parent"),
        "~/.ctx/bin"
    );
    assert_eq!(
        remote_ctx_bin_parent_dir("/opt/ctx/bin/ctx").expect("absolute path parent"),
        "/opt/ctx/bin"
    );
}

#[test]
fn remote_daemon_exec_command_does_not_inject_system_sandbox_cli_env() {
    let command = super::install::render_remote_daemon_exec_cmd("~/.ctx/bin/ctx", 44199, "~/.ctx")
        .expect("render remote daemon exec command");
    assert!(
        command.contains(
            "\"$HOME/.ctx/bin/ctx\" serve --bind 127.0.0.1:44199 --data-dir \"$HOME/.ctx\""
        ),
        "unexpected command: {command}"
    );
    assert!(
        !command.contains("CTX_HARNESS_SANDBOX_CLI_PATH"),
        "remote daemon start command should not inject a system sandbox CLI path: {command}"
    );
    assert!(
        !command.contains("CTX_SANDBOX_PREFETCH"),
        "remote daemon start command should not depend on legacy sandbox prefetch env: {command}"
    );
}

#[test]
fn remote_startup_prewarm_request_targets_daemon_launch_api() {
    let request = super::commands::build_remote_startup_prewarm_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/execution/launch/start");
    assert_eq!(
        request.headers,
        vec![("Content-Type".to_string(), "application/json".to_string())]
    );
    assert_eq!(
        request.body.as_deref(),
        Some(r#"{"kind":"startup_prewarm","prewarm_scope":"all"}"#)
    );
}

#[test]
fn remote_bootstrap_planner_covers_primary_paths() {
    assert_eq!(
        plan_remote_bootstrap(RemoteBootstrapPlannerInput {
            start_remote: true,
            no_start_remote: false,
            existing_daemon_reachable: true,
            managed_binary_present: false,
        }),
        RemoteBootstrapPlan::ConnectToRunningDaemon
    );
    assert_eq!(
        plan_remote_bootstrap(RemoteBootstrapPlannerInput {
            start_remote: false,
            no_start_remote: false,
            existing_daemon_reachable: false,
            managed_binary_present: true,
        }),
        RemoteBootstrapPlan::RefuseBecauseStartRemoteDisabled
    );
    assert_eq!(
        plan_remote_bootstrap(RemoteBootstrapPlannerInput {
            start_remote: true,
            no_start_remote: false,
            existing_daemon_reachable: false,
            managed_binary_present: true,
        }),
        RemoteBootstrapPlan::StartManagedDaemon
    );
    assert_eq!(
        plan_remote_bootstrap(RemoteBootstrapPlannerInput {
            start_remote: true,
            no_start_remote: false,
            existing_daemon_reachable: false,
            managed_binary_present: false,
        }),
        RemoteBootstrapPlan::InstallManagedDaemonThenStart
    );
}

#[test]
fn ssh_connect_job_registry_tracks_phase_and_consumes_terminal_state() {
    let job_id = begin_connect_job().expect("job should start");
    record_connect_job_phase(&job_id, ConnectJobPhase::Planning);
    let pending = desktop_connect_ssh_poll(DesktopSshConnectPollReq {
        job_id: job_id.clone(),
        consume: false,
    })
    .expect("pending snapshot");
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.phase.as_deref(), Some("planning"));
    complete_connect_job_failure(&job_id, "boom".to_string());
    let failed = desktop_connect_ssh_poll(DesktopSshConnectPollReq {
        job_id: job_id.clone(),
        consume: true,
    })
    .expect("failed snapshot");
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.phase.as_deref(), Some("failed"));
    let missing = desktop_connect_ssh_poll(DesktopSshConnectPollReq {
        job_id,
        consume: false,
    });
    assert!(missing.is_err(), "terminal consume should remove the job");
}

#[test]
#[cfg(unix)]
fn bootstrap_failure_cleanup_kills_ephemeral_tunnel() {
    let child = Command::new("sh")
        .arg("-c")
        .arg("sleep 60")
        .spawn()
        .expect("spawn ssh tunnel fixture");
    let pid = child.id();
    assert!(
        pid_is_alive(pid),
        "ephemeral tunnel fixture should start alive"
    );

    let tunnel = TunnelHandle::from_child_for_test(45123, child);
    let err = super::connect::cleanup_ephemeral_tunnel_on_error::<()>(tunnel, anyhow!("boom"))
        .expect_err("cleanup should return the original failure");

    assert!(err.to_string().contains("boom"));
    assert!(
        wait_for_pid_exit(pid, std::time::Duration::from_secs(3)),
        "ephemeral tunnel pid {pid} should be terminated on bootstrap failure"
    );
}

#[test]
fn windows_detection_helpers_match_expected_tokens() {
    assert!(parse_remote_platform_probe_stdout(
        "welcome\n__CTX_PLATFORM_OS__Linux\n__CTX_PLATFORM_ARCH__x86_64\n"
    )
    .is_some());
}

#[test]
fn ssh_auth_failure_detection_matches_permission_denied_errors() {
    assert!(looks_like_ssh_auth_failure(
        "ssh failed to probe remote platform: Permission denied (publickey,password)."
    ));
    assert!(!looks_like_ssh_auth_failure(
        "ssh: connect to host devbox.example port 22: Operation timed out"
    ));
}

#[test]
fn ssh_bootstrap_authorized_keys_command_is_idempotent() {
    let cmd = ssh_authorized_keys_install_command();
    assert!(cmd.contains("grep -qxF \"$key\" \"$HOME/.ssh/authorized_keys\" ||"));
}

#[test]
fn reap_password_once_child_after_input_error_reports_remote_output() {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("echo remote-ssh-failed 1>&2 & exit /b 19");
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-lc").arg("echo remote-ssh-failed >&2; exit 19");
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().expect("spawn failure fixture");
    let err = reap_password_once_child_after_input_error(child, "stdin write failed");
    let msg = err.to_string();
    assert!(msg.contains("stdin write failed"));
    assert!(msg.contains("password-once ssh exited with status"));
    assert!(msg.contains("remote-ssh-failed"));
}

#[test]
fn ssh_config_override_normalization() {
    assert_eq!(
        normalized_ssh_config_override(Some(" /tmp/ctx-fixture-ssh-config ")),
        Some("/tmp/ctx-fixture-ssh-config".to_string())
    );
    assert_eq!(normalized_ssh_config_override(Some("   ")), None);
}

#[test]
fn ssh_identity_file_parser_extracts_and_normalizes_paths() {
    let expanded = "host fixture\nidentityfile ~/.ssh/fixture_key\nidentityfile /tmp/ctx\\ fixture/id_ed25519\n";
    let identities = parse_ssh_identity_files_from_expanded_config(expanded);
    assert_eq!(identities.len(), 2);
    assert_eq!(
        identities[0],
        expand_tilde("~/.ssh/fixture_key").expect("home expansion should succeed")
    );
    assert_eq!(identities[1], PathBuf::from("/tmp/ctx fixture/id_ed25519"));
}

#[test]
fn ssh_identity_path_helpers_strip_and_append_pub_suffix() {
    let public_identity = PathBuf::from("/tmp/ctx-fixture/id_ed25519.pub");
    let private_identity = private_key_path_for_identity(&public_identity);
    assert_eq!(
        private_identity,
        PathBuf::from("/tmp/ctx-fixture/id_ed25519")
    );
    assert_eq!(
        public_key_path_for_private_key(&private_identity),
        public_identity
    );
}

#[test]
fn remote_path_helpers_round_trip() {
    assert_eq!(split_remote_path(""), ("~".to_string(), String::new()));
    assert_eq!(join_remote_path("~", "repo"), "~/repo".to_string());
    assert_eq!(join_remote_path("/", "repo"), "/repo".to_string());
}

#[test]
fn update_channel_validation() {
    assert_eq!(normalize_update_channel(None).expect("default"), "stable");
    assert!(normalize_update_channel(Some("bad channel")).is_err());
}

#[test]
fn remote_update_reuses_existing_managed_binary_when_recorded_active_is_missing() {
    let decision = super::update::resolve_remote_update_target_ctx_bin(
        Some("/tmp/custom/ctx".to_string()),
        MANAGED_REMOTE_CTX_BIN,
        false,
        true,
    );
    assert_eq!(decision.ctx_bin, MANAGED_REMOTE_CTX_BIN);
    assert!(!decision.install_managed);
}

#[test]
fn remote_stop_command_requires_pkill_success() {
    let cmd = remote_stop_daemon_cmd(44199, "/opt/ctx/bin/ctx");
    assert!(cmd.contains("lsof -tiTCP:44199 -sTCP:LISTEN"));
    assert!(cmd.contains("command -v pkill"));
    assert!(cmd.contains("ctx serve"));
}
