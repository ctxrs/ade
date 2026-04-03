use super::*;

struct EnvVarGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

#[test]
fn demo_commands_enabled_respects_env_flag() {
    let _guard = EnvVarGuard::set("CTX_DESKTOP_ALLOW_DEMO_COMMANDS", "1");
    assert!(demo_commands_enabled());
}

#[cfg(unix)]
fn spawn_detached_sleep_pid() -> u32 {
    let output = Command::new("sh")
        .arg("-c")
        .arg("sleep 30 >/dev/null 2>&1 & echo $!")
        .output()
        .expect("spawn detached sleep");
    assert!(
        output.status.success(),
        "detached sleep spawn failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .parse::<u32>()
        .expect("parse detached sleep pid")
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    !pid_is_alive(pid)
}

#[cfg(unix)]
fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    path.exists()
}

#[cfg(unix)]
fn spawn_tokio_sleep_child() -> Child {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 30 >/dev/null 2>&1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().expect("spawn tokio sleep child")
}

#[cfg(unix)]
fn spawn_term_trap_child(term_marker: &Path) -> Child {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("trap 'printf term > \"$CTX_TEST_TERM_MARKER\"; exit 0' TERM; while :; do sleep 1; done")
        .env("CTX_TEST_TERM_MARKER", term_marker)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().expect("spawn term trap child")
}

#[test]
#[cfg(unix)]
fn disconnect_stops_reattached_compatible_local_daemon_pid() {
    let pid = spawn_detached_sleep_pid();
    assert!(
        pid_is_alive(pid),
        "sleep process should be alive before disconnect"
    );

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65531".to_string(),
        "token".to_string(),
        Some(pid),
        LocalConnectionSource::ExistingCompatibleDaemon,
    );
    manager.disconnect();

    assert!(
        wait_for_pid_exit(pid, Duration::from_secs(3)),
        "reattached compatible local daemon pid {pid} should be terminated on disconnect"
    );
}

#[test]
#[cfg(unix)]
fn disconnect_reattached_compatible_local_daemon_prefers_graceful_shutdown() {
    let term_marker =
        std::env::temp_dir().join(format!("ctx-daemon-term-marker-{}", uuid::Uuid::new_v4()));
    let mut child = spawn_term_trap_child(&term_marker);
    let pid = child.id();
    assert!(
        pid_is_alive(pid),
        "term trap child should be alive before disconnect"
    );

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65532".to_string(),
        "token".to_string(),
        Some(pid),
        LocalConnectionSource::ExistingCompatibleDaemon,
    );
    manager.disconnect();

    assert!(
        wait_for_file(&term_marker, Duration::from_secs(3)),
        "term marker should be written after graceful TERM shutdown"
    );
    let _ = child.wait().expect("wait term trap child");
    let marker = std::fs::read_to_string(&term_marker)
        .expect("term marker should be written by graceful TERM handler");
    assert_eq!(marker, "term");
    std::fs::remove_file(&term_marker).ok();
}

#[test]
#[cfg(unix)]
fn disconnect_does_not_stop_env_override_local_daemon_pid() {
    let pid = spawn_detached_sleep_pid();
    assert!(
        pid_is_alive(pid),
        "sleep process should be alive before disconnect"
    );

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65530".to_string(),
        "token".to_string(),
        Some(pid),
        LocalConnectionSource::EnvOverride,
    );
    manager.disconnect();

    assert!(
        pid_is_alive(pid),
        "env override local daemon pid {pid} must not be terminated by disconnect"
    );
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .output();
}

#[test]
#[cfg(unix)]
fn replacing_env_override_local_connection_leaves_previous_pid_running() {
    let previous_pid = spawn_detached_sleep_pid();
    assert!(
        pid_is_alive(previous_pid),
        "previous pid should start alive"
    );

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65527".to_string(),
        "token".to_string(),
        Some(previous_pid),
        LocalConnectionSource::EnvOverride,
    );
    manager.set_local_attached(
        "http://127.0.0.1:65526".to_string(),
        "token".to_string(),
        None,
        LocalConnectionSource::EnvOverride,
    );

    assert!(
        pid_is_alive(previous_pid),
        "replacing an env override pid {previous_pid} must not terminate it"
    );
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(previous_pid.to_string())
        .output();
}

#[test]
#[cfg(unix)]
fn replacing_local_connection_stops_previous_child() {
    let previous = spawn_tokio_sleep_child();
    let previous_pid = previous.id();
    assert!(
        pid_is_alive(previous_pid),
        "previous local child should start alive"
    );

    let next = spawn_tokio_sleep_child();
    let next_pid = next.id();
    assert!(
        pid_is_alive(next_pid),
        "next local child should start alive"
    );

    let manager = ConnectionManager::default();
    manager.set_local(
        "http://127.0.0.1:65525".to_string(),
        "token".to_string(),
        previous,
        false,
    );
    manager.set_local(
        "http://127.0.0.1:65524".to_string(),
        "token".to_string(),
        next,
        false,
    );

    assert!(
        wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
        "replaced local child {previous_pid} should be terminated"
    );
    assert!(
        pid_is_alive(next_pid),
        "replacement local child {next_pid} should remain alive until disconnect"
    );

    manager.disconnect();
    assert!(
        wait_for_pid_exit(next_pid, Duration::from_secs(3)),
        "active replacement local child {next_pid} should be terminated on disconnect"
    );
}

#[test]
#[cfg(unix)]
fn disconnect_owned_child_local_daemon_prefers_graceful_shutdown() {
    let term_marker = std::env::temp_dir().join(format!(
        "ctx-owned-child-term-marker-{}",
        uuid::Uuid::new_v4()
    ));
    let child = spawn_term_trap_child(&term_marker);
    let pid = child.id();
    assert!(
        pid_is_alive(pid),
        "owned child term trap should be alive before disconnect"
    );

    let manager = ConnectionManager::default();
    manager.set_local(
        "http://127.0.0.1:65523".to_string(),
        "token".to_string(),
        child,
        false,
    );
    manager.disconnect();

    assert!(
        wait_for_pid_exit(pid, Duration::from_secs(3)),
        "owned child local daemon pid {pid} should exit after disconnect"
    );
    let marker = std::fs::read_to_string(&term_marker)
        .expect("owned child term marker should be written by graceful TERM handler");
    assert_eq!(marker, "term");
    std::fs::remove_file(&term_marker).ok();
}

#[test]
#[cfg(unix)]
fn reattaching_to_same_owned_local_daemon_preserves_process() {
    let child = spawn_tokio_sleep_child();
    let pid = child.id();
    assert!(pid_is_alive(pid), "local child should start alive");

    let manager = ConnectionManager::default();
    manager.set_local(
        "http://127.0.0.1:65524".to_string(),
        "token".to_string(),
        child,
        false,
    );
    manager.set_local_attached(
        "http://127.0.0.1:65524".to_string(),
        "token".to_string(),
        Some(pid),
        LocalConnectionSource::ExistingCompatibleDaemon,
    );

    assert!(
        pid_is_alive(pid),
        "same-daemon handoff must not kill the process being reattached"
    );

    manager.disconnect();
    assert!(
        wait_for_pid_exit(pid, Duration::from_secs(3)),
        "reattached owned local daemon pid {pid} should still be terminated on disconnect"
    );
}

#[test]
#[cfg(unix)]
fn replacing_ssh_connection_stops_previous_tunnel() {
    let previous = spawn_tokio_sleep_child();
    let previous_pid = previous.id();
    assert!(
        pid_is_alive(previous_pid),
        "previous ssh tunnel should start alive"
    );

    let next = spawn_tokio_sleep_child();
    let next_pid = next.id();
    assert!(pid_is_alive(next_pid), "next ssh tunnel should start alive");

    let manager = ConnectionManager::default();
    manager.set_ssh(
        "http://127.0.0.1:65523".to_string(),
        Some("token".to_string()),
        previous,
        "example.test".to_string(),
        Some("dev".to_string()),
        22,
        Some("/tmp/ctx".to_string()),
        SshRuntimeMetadata {
            managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
            active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            ssh_password_once: None,
            admin_password_once: None,
        },
    );
    manager.set_ssh(
        "http://127.0.0.1:65522".to_string(),
        Some("token".to_string()),
        next,
        "example.test".to_string(),
        Some("dev".to_string()),
        22,
        Some("/tmp/ctx".to_string()),
        SshRuntimeMetadata {
            managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
            active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            ssh_password_once: None,
            admin_password_once: None,
        },
    );

    assert!(
        wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
        "replaced ssh tunnel {previous_pid} should be terminated"
    );
    assert!(
        pid_is_alive(next_pid),
        "replacement ssh tunnel {next_pid} should remain alive until disconnect"
    );

    manager.disconnect();
    assert!(
        wait_for_pid_exit(next_pid, Duration::from_secs(3)),
        "active replacement ssh tunnel {next_pid} should be terminated on disconnect"
    );
}

#[test]
#[cfg(unix)]
fn replace_with_ssh_defers_previous_tunnel_cleanup_to_caller() {
    let previous = spawn_tokio_sleep_child();
    let previous_pid = previous.id();
    assert!(
        pid_is_alive(previous_pid),
        "previous ssh tunnel should start alive"
    );

    let next = spawn_tokio_sleep_child();
    let next_pid = next.id();
    assert!(pid_is_alive(next_pid), "next ssh tunnel should start alive");

    let manager = ConnectionManager::default();
    manager.set_ssh(
        "http://127.0.0.1:65523".to_string(),
        Some("token".to_string()),
        previous,
        "example.test".to_string(),
        Some("dev".to_string()),
        22,
        Some("/tmp/ctx".to_string()),
        SshRuntimeMetadata {
            managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
            active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            ssh_password_once: None,
            admin_password_once: None,
        },
    );

    let displaced = manager.replace_with_ssh(
        "http://127.0.0.1:65522".to_string(),
        Some("token".to_string()),
        next,
        "example.test".to_string(),
        Some("dev".to_string()),
        22,
        Some("/tmp/ctx".to_string()),
        SshRuntimeMetadata {
            managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
            active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            ssh_password_once: None,
            admin_password_once: None,
        },
    );

    assert!(displaced.is_some(), "previous tunnel should be returned");
    assert!(
        pid_is_alive(previous_pid),
        "replace_with_ssh should not eagerly terminate the displaced tunnel"
    );
    assert!(
        pid_is_alive(next_pid),
        "replacement tunnel should remain alive after the swap"
    );

    cleanup_active_connection(displaced.expect("previous connection should exist"));
    assert!(
        wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
        "caller cleanup should terminate displaced tunnel {previous_pid}"
    );

    manager.disconnect();
    assert!(
        wait_for_pid_exit(next_pid, Duration::from_secs(3)),
        "active replacement ssh tunnel {next_pid} should be terminated on disconnect"
    );
}

#[test]
fn daemon_request_error_includes_method_and_url_context() {
    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65535".to_string(),
        "token".to_string(),
        None,
        LocalConnectionSource::ExistingCompatibleDaemon,
    );
    let err = manager
        .daemon_request(DesktopDaemonRequest {
            method: "GET".to_string(),
            path: "/api/health".to_string(),
            body: None,
            headers: Vec::new(),
        })
        .expect_err("request should fail on closed port");
    let message = format!("{err:#}");
    assert!(
        message.contains("sending request GET http://127.0.0.1:65535/api/health"),
        "expected method/url context in error, got: {message}"
    );
}

#[test]
fn daemon_request_reuses_connection_http_client() {
    reset_connection_http_client_build_count();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
            )
            .expect("write response");
        }
    });

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        format!("http://{}", addr),
        "token".to_string(),
        None,
        LocalConnectionSource::EnvOverride,
    );

    for _ in 0..2 {
        let response = manager
            .daemon_request(DesktopDaemonRequest {
                method: "GET".to_string(),
                path: "/api/health".to_string(),
                body: None,
                headers: Vec::new(),
            })
            .expect("daemon request succeeds");
        assert_eq!(response.status, 200);
    }

    server.join().expect("join test server");
    assert_eq!(connection_http_client_build_count(), 1);
}
