use super::*;

#[test]
fn parse_guest_exec_env_rejects_reserved_helper_keys() {
    let err = parse_guest_exec_env(&["CTX_AVF_SECRET=1".to_string()])
        .expect_err("reserved helper keys should be rejected");
    assert!(err.to_string().contains("reserved"));
}

#[test]
fn runtime_without_guest_agent_stays_simulated() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-capability-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(temp.join("helpers")).expect("create helpers dir");
    let (enabled, note) = shared_vm_runtime_supports_real_guest_exec(&temp);
    assert!(!enabled);
    assert!(note.contains("missing"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn runtime_with_guest_agent_can_enable_real_vm_path() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-capability-agent-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    let helper_path = temp.join("helpers").join(AVF_LINUX_GUEST_AGENT_HELPER);
    fs::create_dir_all(helper_path.parent().expect("helper parent")).expect("helpers dir");
    fs::write(&helper_path, b"guest-agent").expect("guest-agent helper");
    let (enabled, note) = shared_vm_runtime_supports_real_guest_exec(&temp);
    assert!(enabled);
    assert!(note.contains("guest-agent payload"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn cloud_init_user_data_embeds_guest_agent_and_service() {
    let user_data = render_shared_vm_cloud_init_user_data(b"guest-agent", Some(b"egress-proxy"));
    assert!(user_data.contains("#cloud-config"));
    assert!(user_data.contains("/usr/local/bin/ctx-avf-linux-guest-agent"));
    assert!(user_data.contains("/usr/local/bin/ctx-egress-proxy"));
    assert!(user_data.contains(SHARED_VM_GUEST_AGENT_SERVICE_NAME));
    assert!(user_data.contains("systemctl enable --now ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("StandardOutput=journal+console"));
    assert!(user_data.contains("starting guest-agent"));
    assert!(user_data.contains("preparing ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("systemctl status ctx-avf-linux-guest-agent.service --no-pager"));
    let content_lines = user_data
        .lines()
        .skip_while(|line| *line != "    content: |")
        .skip(1)
        .take_while(|line| !line.starts_with("  - path: "))
        .collect::<Vec<_>>();
    assert!(!content_lines.is_empty());
    assert!(content_lines.iter().all(|line| line.starts_with("      ")));
}

#[test]
fn cloud_init_meta_data_changes_when_guest_payload_changes() {
    let first = render_shared_vm_cloud_init_meta_data(b"guest-agent-a", Some(b"egress-proxy-a"));
    let second = render_shared_vm_cloud_init_meta_data(b"guest-agent-b", Some(b"egress-proxy-b"));
    assert!(first.contains("instance-id: ctx-avf-linux-"));
    assert_ne!(first, second);
}

#[cfg(unix)]
#[test]
fn guest_exec_relays_request_over_shared_vm_control_socket() {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let runtime_root = temp.join("runtime");
    fs::create_dir_all(&runtime_root).expect("runtime dir");
    let rootfs = runtime_root.join("rootfs.img");
    let kernel = runtime_root.join("kernel");
    let initrd = runtime_root.join("initrd");
    fs::write(&rootfs, b"rootfs").expect("rootfs");
    fs::write(&kernel, b"kernel").expect("kernel");
    fs::write(&initrd, b"initrd").expect("initrd");
    start_shared_vm(
        &temp,
        &runtime_root,
        &rootfs,
        &kernel,
        &initrd,
        "test".into(),
    )
    .expect("start shared vm");

    let metadata_path = shared_vm_worktree_metadata_path(&temp, "ws-123", "wt-456");
    persist_guest_worktree_state(
        &metadata_path,
        &PersistedGuestWorktreeState {
            workspace_id: "ws-123".to_string(),
            worktree_id: "wt-456".to_string(),
            host_workspace_root: temp.join("repo"),
            guest_root: PathBuf::from("/ctx/ws/worktrees/wt-456"),
            host_shadow_root: temp.join("shadow-root"),
            guest_user: "ctx-ws-test".to_string(),
            base_commit_sha: "abc123".to_string(),
            branch_name: "ctx/ws-123/wt-456".to_string(),
            updated_at: now_timestamp_string(),
            simulated: true,
            notes: vec![],
        },
    )
    .expect("persist guest worktree state");

    let socket_path = shared_vm_control_socket_path(&temp);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).expect("socket dir");
    }
    if socket_path.exists() {
        fs::remove_file(&socket_path).expect("remove stale socket");
    }
    let listener = UnixListener::bind(&socket_path).expect("bind control socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept control socket");
        let request = read_exec_frame(&mut stream)
            .expect("read request")
            .expect("request frame");
        let request = match request {
            AvfLinuxExecFrame::Request(request) => request,
            other => panic!("expected request frame, got {other:?}"),
        };
        assert_eq!(request.command, "/usr/bin/env");
        assert_eq!(request.args, vec!["--version".to_string()]);
        assert_eq!(request.cwd, "/ctx/ws/worktrees/wt-456/src");
        assert_eq!(request.user.as_deref(), Some("ctxagent"));
        assert!(request.pty);
        assert_eq!(
            request.env.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 7 }),
        )
        .expect("write exit frame");
    });

    let exit_code = guest_exec(
        &temp,
        "ws-123",
        "wt-456",
        Path::new("/ctx/ws/worktrees/wt-456/src"),
        "/usr/bin/env",
        &["TERM=xterm-256color".to_string()],
        Some("ctxagent"),
        true,
        &["--version".to_string()],
    )
    .expect("guest exec should succeed");

    assert_eq!(exit_code, 7);
    server.join().expect("server thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(unix)]
#[test]
fn shared_vm_control_connection_proxies_to_guest_agent() {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-proxy-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");

    let metadata_path = shared_vm_worktree_metadata_path(&temp, "ws-123", "wt-456");
    let host_shadow_root = temp.join("shadow-root");
    fs::create_dir_all(host_shadow_root.join("src")).expect("shadow root");
    persist_guest_worktree_state(
        &metadata_path,
        &PersistedGuestWorktreeState {
            workspace_id: "ws-123".to_string(),
            worktree_id: "wt-456".to_string(),
            host_workspace_root: temp.join("repo"),
            guest_root: PathBuf::from("/ctx/ws/worktrees/wt-456"),
            host_shadow_root: host_shadow_root.clone(),
            guest_user: "ctx-ws-test".to_string(),
            base_commit_sha: "abc123".to_string(),
            branch_name: "ctx/ws-123/wt-456".to_string(),
            updated_at: now_timestamp_string(),
            simulated: true,
            notes: vec![],
        },
    )
    .expect("persist guest worktree state");

    let agent_socket = shared_vm_guest_agent_socket_path(&temp);
    if let Some(parent) = agent_socket.parent() {
        fs::create_dir_all(parent).expect("socket dir");
    }
    if agent_socket.exists() {
        fs::remove_file(&agent_socket).expect("remove stale guest-agent socket");
    }
    let listener = UnixListener::bind(&agent_socket).expect("bind guest-agent socket");
    let agent = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept guest-agent connection");
        let request = read_exec_frame(&mut stream)
            .expect("read proxied request")
            .expect("request frame");
        let request = match request {
            AvfLinuxExecFrame::Request(request) => request,
            other => panic!("expected proxied request frame, got {other:?}"),
        };
        assert_eq!(request.command, "/usr/bin/env");
        assert_eq!(
            request.cwd,
            host_shadow_root.join("src").display().to_string()
        );
        assert_eq!(request.args, vec!["--version".to_string()]);
        assert_eq!(
            request.env.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 13 }),
        )
        .expect("write exit frame");
    });

    let (mut client, server) = UnixStream::pair().expect("unix stream pair");
    let temp_for_server = temp.clone();
    let relay = thread::spawn(move || {
        handle_shared_vm_control_connection(&temp_for_server, server)
            .expect("proxy shared vm control connection");
    });

    write_exec_frame(
        &mut client,
        &AvfLinuxExecFrame::Request(AvfLinuxExecRequest::new(
            "/usr/bin/env",
            vec!["--version".to_string()],
            "/ctx/ws/worktrees/wt-456/src",
            Some("ctxagent".to_string()),
            HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
            false,
        )),
    )
    .expect("write client request");

    let frame = read_exec_frame(&mut client)
        .expect("read proxied exit")
        .expect("exit frame");
    assert_eq!(
        frame,
        AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 13 })
    );

    relay.join().expect("relay thread");
    agent.join().expect("agent thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}
