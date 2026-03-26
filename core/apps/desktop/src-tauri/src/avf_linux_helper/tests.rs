use super::*;
use serde_yaml::Value;

fn git(args: &[&str], cwd: &Path) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

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
fn stage_shadow_root_from_host_workspace_copies_standalone_repo() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-shadow-copy-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    let repo_root = temp.join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    git(&["init", "-b", "main"], &repo_root);
    git(&["config", "user.email", "test@example.com"], &repo_root);
    git(&["config", "user.name", "Test User"], &repo_root);
    fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], &repo_root);
    git(&["commit", "-m", "initial"], &repo_root);

    let shadow_root = temp.join("shadow-root");
    stage_shadow_root_from_host_workspace(&repo_root, &shadow_root)
        .expect("stage shadow root from standalone repo");

    assert!(shadow_root.join("README.md").exists());
    assert!(shadow_root.join(".git").is_dir());
    git(&["rev-parse", "--is-inside-work-tree"], &shadow_root);
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn cloud_init_user_data_embeds_guest_agent_and_service() {
    let user_data = render_shared_vm_cloud_init_user_data(
        Path::new("/tmp"),
        b"guest-agent",
        Some(&b"egress-proxy"[..]),
        Path::new("/tmp/runtime/helpers/container-stack.tar.gz"),
        "deadbeef",
    );
    assert!(user_data.contains("#cloud-config"));
    assert!(user_data.contains("/usr/local/bin/ctx-avf-linux-guest-agent"));
    assert!(user_data.contains("/usr/local/bin/ctx-egress-proxy"));
    assert!(user_data.contains(SHARED_VM_GROW_ROOTFS_INSTALL_PATH));
    assert!(user_data.contains("/usr/local/lib/ctx/ctx-avf-install-container-stack.sh"));
    assert!(user_data.contains(SHARED_VM_GROW_ROOTFS_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_GUEST_AGENT_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_CONTAINERD_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_BUILDKIT_SERVICE_NAME));
    assert!(user_data.contains("systemctl enable --now ctx-avf-grow-rootfs.service"));
    assert!(user_data.contains(&format!(
        "systemctl enable --now {service_name}",
        service_name = SHARED_VM_HOST_DATA_SERVICE_NAME
    )));
    assert!(user_data.contains("systemctl enable --now ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("systemctl enable --now containerd.service"));
    assert!(user_data.contains("systemctl enable --now buildkit.service"));
    assert!(user_data.contains("StandardOutput=journal+console"));
    assert!(user_data.contains("starting guest-agent"));
    assert!(user_data.contains("preparing ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("systemctl status ctx-avf-linux-guest-agent.service --no-pager"));
    assert!(user_data.contains("/tmp/runtime/helpers/container-stack.tar.gz"));
    let parsed: Value = serde_yaml::from_str(&user_data).expect("cloud-init YAML should parse");
    let runcmd = parsed["runcmd"]
        .as_sequence()
        .expect("cloud-init runcmd should be a sequence");
    assert_eq!(runcmd.len(), 8);
    assert_eq!(
        runcmd[0]
            .as_sequence()
            .expect("first runcmd entry should be daemon reload"),
        &vec![Value::from("systemctl"), Value::from("daemon-reload")]
    );
    let host_data_enable_index = user_data
        .find(&format!(
            "systemctl enable --now {service_name}",
            service_name = SHARED_VM_HOST_DATA_SERVICE_NAME
        ))
        .expect("host-data enable command");
    assert!(runcmd[3]
        .as_str()
        .expect("prepare guest-agent step should be a string")
        .contains("preparing ctx-avf-linux-guest-agent.service"));
    assert!(runcmd[4]
        .as_str()
        .expect("container-stack install step should be a string")
        .contains(SHARED_VM_GUEST_CONTAINER_STACK_INSTALL_PATH));
    assert!(
        host_data_enable_index
            < user_data
                .find("preparing ctx-avf-linux-guest-agent.service")
                .expect("prepare guest-agent command")
    );
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
fn materialize_writable_rootfs_image_expands_small_rootfs() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-rootfs-grow-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let source_rootfs = temp.join("source-rootfs.raw");
    let source_file = File::create(&source_rootfs).expect("create source rootfs");
    source_file
        .set_len(1024 * 1024)
        .expect("seed source rootfs");
    drop(source_file);

    let (staged_rootfs, note) =
        materialize_writable_rootfs_image(&temp, &source_rootfs).expect("materialize rootfs");

    assert_eq!(staged_rootfs, shared_vm_rootfs_path(&temp));
    assert_eq!(
        fs::metadata(&staged_rootfs).expect("rootfs metadata").len(),
        SHARED_VM_MIN_WRITABLE_ROOTFS_BYTES
    );
    let note = note.expect("growth note");
    assert!(note.contains("expanded writable AVF Linux rootfs"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_guest_readiness_args_include_bridge_probe() {
    let rendered = shared_vm_guest_readiness_args().join(" ");
    assert!(rendered.contains("bridge_probe_failed"));
    assert!(rendered.contains("ip link add name \"$probe_bridge\" type bridge"));
    assert!(rendered.contains(SHARED_VM_GUEST_NERDCTL_BIN));
    assert!(rendered.contains(SHARED_VM_GUEST_BUILDKITCTL_BIN));
}

#[test]
fn cold_boot_timeout_extends_when_rootfs_is_materialized() {
    assert_eq!(
        real_guest_exec_ready_timeout_for_rootfs_materialization(None),
        default_real_guest_exec_ready_timeout()
    );
    assert_eq!(
        real_guest_exec_ready_timeout_for_rootfs_materialization(Some(
            "copied rootfs image into helper-managed writable path"
        )),
        cold_boot_real_guest_exec_ready_timeout()
    );
}

#[test]
fn bridge_probe_failure_triggers_writable_rootfs_reset() {
    let err = anyhow::anyhow!(
        "guest exec readiness probe exited 41 (stdout='', stderr='[ctx-avf-linux] bridge_probe_failed')"
    );
    assert!(shared_vm_readiness_failure_requires_writable_rootfs_reset(
        &err
    ));
    let unrelated = anyhow::anyhow!("guest exec readiness probe exited 1");
    assert!(!shared_vm_readiness_failure_requires_writable_rootfs_reset(
        &unrelated
    ));
}

#[test]
fn cloud_init_enables_host_data_before_touching_host_payload() {
    let user_data = render_shared_vm_cloud_init_user_data(
        Path::new("/tmp"),
        b"guest-agent",
        None,
        Path::new("/tmp/runtime/helpers/container-stack.tar.gz"),
        "deadbeef",
    );
    let daemon_reload_index = user_data
        .find("- [ systemctl, daemon-reload ]")
        .expect("daemon reload command");
    let host_data_enable_index = user_data
        .find(&format!(
            "systemctl enable --now {service_name}",
            service_name = SHARED_VM_HOST_DATA_SERVICE_NAME
        ))
        .expect("host-data enable command");
    let prepare_guest_agent_index = user_data
        .find("preparing ctx-avf-linux-guest-agent.service")
        .expect("prepare guest-agent command");
    assert!(daemon_reload_index < host_data_enable_index);
    assert!(host_data_enable_index < prepare_guest_agent_index);
}

#[test]
fn resetting_writable_runtime_state_removes_only_derived_files() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-reset-runtime-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    let control_socket = shared_vm_control_socket_path(&temp);
    let guest_agent_socket = shared_vm_guest_agent_socket_path(&temp);
    let saved_state = shared_vm_saved_state_path(&temp);
    let rootfs = shared_vm_rootfs_path(&temp);
    for path in [&control_socket, &guest_agent_socket, &saved_state, &rootfs] {
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, b"x").expect("seed file");
    }

    reset_writable_shared_vm_runtime_state(&temp).expect("reset runtime state");

    for path in [&control_socket, &guest_agent_socket, &saved_state, &rootfs] {
        assert!(!path.exists(), "{} should be removed", path.display());
    }
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn cloud_init_meta_data_changes_when_guest_payload_changes() {
    let first = render_shared_vm_cloud_init_meta_data(
        b"guest-agent-a",
        Some(b"egress-proxy-a"),
        "container-stack-a",
    );
    let second = render_shared_vm_cloud_init_meta_data(
        b"guest-agent-b",
        Some(b"egress-proxy-b"),
        "container-stack-b",
    );
    assert!(first.contains("instance-id: ctx-avf-linux-"));
    assert_ne!(first, second);
}

#[cfg(target_os = "macos")]
#[test]
fn transient_guest_control_connect_nserrors_retry() {
    assert!(is_transient_guest_control_connect_nserror(
        "NSPOSIXErrorDomain",
        libc::ECONNRESET as isize
    ));
    assert!(is_transient_guest_control_connect_nserror(
        "NSPOSIXErrorDomain",
        libc::ECONNREFUSED as isize
    ));
    assert!(!is_transient_guest_control_connect_nserror(
        "NSPOSIXErrorDomain",
        libc::ENOENT as isize
    ));
    assert!(!is_transient_guest_control_connect_nserror(
        "SomeOtherDomain",
        libc::ECONNRESET as isize
    ));
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
fn guest_exec_capture_reports_explicit_error_frames() {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-capture-error-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let socket_path = temp.join("shared-vm-control.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind control socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept control socket");
        let frame = read_exec_frame(&mut stream)
            .expect("read request")
            .expect("request frame");
        assert!(matches!(frame, AvfLinuxExecFrame::Request(_)));
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                code: "guest_control_connect_failed".to_string(),
                message: "connecting to guest vsock port 47001: transient connect error"
                    .to_string(),
            }),
        )
        .expect("write explicit error frame");
    });

    let result = run_guest_exec_capture(
        &socket_path,
        Path::new("/"),
        "/usr/bin/true",
        &[],
        Some("root"),
        HashMap::new(),
        None,
    );
    let err = match result {
        Ok(_) => panic!("explicit error frame should fail the capture"),
        Err(err) => err,
    };

    let rendered = err.to_string();
    assert!(rendered.contains("guest exec failed"));
    assert!(rendered.contains("guest_control_connect_failed"));
    assert!(rendered.contains("connecting to guest vsock port 47001"));

    server.join().expect("server thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(all(target_os = "macos", unix))]
#[test]
fn shared_vm_relay_turns_truncated_guest_frames_into_explicit_error_frames() {
    use std::os::fd::{FromRawFd, IntoRawFd};
    use std::thread;

    let (mut client, relay_client) = UnixStream::pair().expect("client pair");
    let (mut guest_server, guest_relay) = UnixStream::pair().expect("guest pair");
    let relay = thread::spawn(move || {
        let guest = unsafe { File::from_raw_fd(guest_relay.into_raw_fd()) };
        relay_shared_vm_control_client(relay_client, guest)
    });

    write_exec_frame(
        &mut client,
        &AvfLinuxExecFrame::Request(AvfLinuxExecRequest::new(
            "/usr/bin/true",
            Vec::new(),
            "/",
            Some("root".to_string()),
            HashMap::new(),
            false,
        )),
    )
    .expect("write request frame");

    let guest = thread::spawn(move || {
        let frame = read_exec_frame(&mut guest_server)
            .expect("read forwarded request")
            .expect("request frame");
        assert!(matches!(frame, AvfLinuxExecFrame::Request(_)));
        guest_server
            .write_all(&[3, 0, 0, 0, 5, b'o', b'k'])
            .expect("write truncated stdout frame");
    });

    let frame = read_exec_frame(&mut client)
        .expect("read explicit transport error")
        .expect("transport error frame");
    let error = match frame {
        AvfLinuxExecFrame::Error(error) => error,
        other => panic!("expected explicit transport error frame, got {other:?}"),
    };
    assert_eq!(error.code, "guest_control_stream_closed");
    assert!(error
        .message
        .contains("reading shared VM guest response frame failed"));
    assert!(
        error.message.contains("failed to fill whole buffer")
            || error.message.contains("unexpected end of file")
    );

    let relay_err = relay
        .join()
        .expect("relay thread join")
        .expect_err("relay should fail after truncated guest frame");
    assert!(relay_err
        .to_string()
        .contains("reading shared VM guest response frame"));

    guest.join().expect("guest thread");
}

#[cfg(unix)]
#[test]
fn non_pty_guest_exec_cli_writes_captured_output() {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-cli-capture-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let socket_path = temp.join("shared-vm-control.sock");
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
        assert_eq!(request.command, "/bin/pwd");
        assert_eq!(request.cwd, "/ctx/ws/worktrees/wt-456");
        assert!(!request.pty);
        assert_eq!(request.user.as_deref(), Some("ctxagent"));
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stdout(b"/ctx/ws/worktrees/wt-456\n".to_vec()),
        )
        .expect("write stdout frame");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stderr(b"warning: capture-path\n".to_vec()),
        )
        .expect("write stderr frame");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 17 }),
        )
        .expect("write exit frame");
    });

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_guest_exec_cli(
        &socket_path,
        Path::new("/ctx/ws/worktrees/wt-456"),
        "/bin/pwd",
        &[],
        Some("ctxagent"),
        HashMap::new(),
        false,
        &mut stdout,
        &mut stderr,
    )
    .expect("non-PTY CLI guest exec should succeed");

    assert_eq!(exit_code, 17);
    assert_eq!(
        String::from_utf8(stdout).expect("stdout utf8"),
        "/ctx/ws/worktrees/wt-456\n"
    );
    assert_eq!(
        String::from_utf8(stderr).expect("stderr utf8"),
        "warning: capture-path\n"
    );
    server.join().expect("server thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(unix)]
#[test]
fn non_pty_guest_exec_cli_forwards_piped_stdin_into_capture_path() {
    use std::io::Cursor;
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-cli-stdin-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let socket_path = temp.join("shared-vm-control.sock");
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
        assert_eq!(request.command, "/usr/bin/tar");
        assert_eq!(request.cwd, "/");
        assert!(!request.pty);

        let stdin = read_exec_frame(&mut stream)
            .expect("read stdin frame")
            .expect("stdin frame");
        let AvfLinuxExecFrame::Stdin(stdin) = stdin else {
            panic!("expected stdin frame");
        };
        assert_eq!(stdin, b"archive-payload".to_vec());

        let close = read_exec_frame(&mut stream)
            .expect("read close frame")
            .expect("close frame");
        assert!(matches!(close, AvfLinuxExecFrame::CloseStdin));

        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stdout(b"imported\n".to_vec()),
        )
        .expect("write stdout frame");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 }),
        )
        .expect("write exit frame");
    });

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdin = Cursor::new(b"archive-payload".to_vec());
    let exit_code = run_guest_exec_cli_with_capture_stdin(
        &socket_path,
        Path::new("/"),
        "/usr/bin/tar",
        &["-xf".to_string(), "-".to_string()],
        None,
        HashMap::new(),
        Some(&mut stdin),
        &mut stdout,
        &mut stderr,
    )
    .expect("non-PTY CLI guest exec with piped stdin should succeed");

    assert_eq!(exit_code, 0);
    assert_eq!(
        String::from_utf8(stdout).expect("stdout utf8"),
        "imported\n"
    );
    assert!(stderr.is_empty());
    server.join().expect("server thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(unix)]
#[test]
fn exec_stream_payload_budget_stays_within_shared_vm_safe_limit() {
    assert!(
        AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD <= 1024,
        "shared-VM exec transport truncated larger stdin frames in live tar-import repros",
    );
}

#[cfg(unix)]
#[test]
fn relay_child_output_splits_large_stdout_frames_below_transport_limit() {
    use std::io::Cursor;

    let payload = vec![b'x'; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD + 33];
    let mut reader = Cursor::new(payload);
    let (mut client, server) = UnixStream::pair().expect("stdout relay pair");

    relay_child_output(&mut reader, Arc::new(Mutex::new(server)), true);

    let first = read_exec_frame(&mut client)
        .expect("read first stdout frame")
        .expect("first stdout frame present");
    let second = read_exec_frame(&mut client)
        .expect("read second stdout frame")
        .expect("second stdout frame present");
    let eof = read_exec_frame(&mut client).expect("read eof");

    let first = match first {
        AvfLinuxExecFrame::Stdout(bytes) => bytes,
        other => panic!("expected first stdout frame, got {other:?}"),
    };
    let second = match second {
        AvfLinuxExecFrame::Stdout(bytes) => bytes,
        other => panic!("expected second stdout frame, got {other:?}"),
    };

    assert_eq!(first.len(), AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD);
    assert_eq!(second.len(), 33);
    assert!(eof.is_none());
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
