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

fn gibibytes(value: u64) -> u64 {
    value * 1024 * 1024 * 1024
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
fn resolve_avf_vm_sizing_defaults_to_host_logical_cpu_count_and_reserved_memory() {
    let sizing = resolve_avf_vm_sizing(
        1,
        16,
        gibibytes(2),
        gibibytes(64),
        12,
        gibibytes(32),
        None,
        None,
    );

    assert_eq!(sizing.cpu_count, 12);
    assert_eq!(sizing.memory_size_bytes, gibibytes(28));
    assert!(sizing.policy_note.contains("host logical CPU count"));
    assert!(sizing
        .policy_note
        .contains("host RAM minus 4096 MiB reserve"));
}

#[test]
fn resolve_avf_vm_sizing_clamps_defaults_to_avf_limits_and_memory_floor() {
    let sizing = resolve_avf_vm_sizing(
        2,
        8,
        gibibytes(2),
        gibibytes(64),
        32,
        gibibytes(6),
        None,
        None,
    );

    assert_eq!(sizing.cpu_count, 8);
    assert_eq!(sizing.memory_size_bytes, gibibytes(4));
}

#[test]
fn resolve_avf_vm_sizing_applies_debug_overrides() {
    let sizing = resolve_avf_vm_sizing(
        1,
        8,
        gibibytes(2),
        gibibytes(10),
        4,
        gibibytes(32),
        Some(6),
        Some(gibibytes(12) + 1234),
    );

    assert_eq!(sizing.cpu_count, 6);
    assert_eq!(sizing.memory_size_bytes, gibibytes(10));
    assert!(sizing.policy_note.contains(SHARED_VM_CPU_COUNT_ENV));
    assert!(sizing
        .policy_note
        .contains(SHARED_VM_MEMORY_CEILING_BYTES_ENV));
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
    assert!(user_data.contains(SHARED_VM_DATA_DISK_INSTALL_PATH));
    assert!(user_data.contains("/usr/local/lib/ctx/ctx-avf-install-container-stack.sh"));
    assert!(user_data.contains(SHARED_VM_DATA_DISK_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_GUEST_AGENT_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_CONTAINERD_SERVICE_NAME));
    assert!(user_data.contains(SHARED_VM_BUILDKIT_SERVICE_NAME));
    assert!(user_data.contains("systemctl enable --now ctx-avf-data-disk.service"));
    assert!(user_data.contains(&format!(
        "systemctl enable --now {service_name}",
        service_name = SHARED_VM_HOST_DATA_SERVICE_NAME
    )));
    assert!(user_data.contains("systemctl enable --now ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("systemctl enable --now containerd.service"));
    assert!(user_data.contains("systemctl enable --now buildkit.service"));
    assert!(user_data.contains("mount_root='/ctx'"));
    assert!(user_data.contains("\"$mount_root/ws/worktrees\""));
    assert!(user_data.contains("\"$mount_root/home\""));
    assert!(user_data.contains("\"$mount_root/cache\""));
    assert!(user_data.contains("\"$mount_root/tmp\""));
    assert!(user_data.contains(".ctx-avf-data-disk-ready"));
    assert!(user_data.contains("\"$mount_root/system/containerd\""));
    assert!(user_data.contains("\"$mount_root/system/buildkit\""));
    assert!(
        user_data.contains("mount --bind \"$mount_root/system/containerd\" /var/lib/containerd")
    );
    assert!(user_data.contains("mount --bind \"$mount_root/system/buildkit\" /var/lib/buildkit"));
    assert!(user_data.contains("StandardOutput=journal+console"));
    assert!(user_data.contains("starting guest-agent"));
    assert!(user_data.contains("CTX_AVF_GUEST_CONTROL_READY_MARKER"));
    assert!(user_data.contains("/tmp/managed/vms/avf-linux"));
    assert!(user_data.contains("guest-control-ready"));
    assert!(user_data.contains("preparing ctx-avf-linux-guest-agent.service"));
    assert!(user_data.contains("systemctl status ctx-avf-linux-guest-agent.service --no-pager"));
    assert!(user_data.contains("/tmp/runtime/helpers/container-stack.tar.gz"));
    assert!(!user_data.contains("ctx-avf-grow-rootfs.service"));
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
    let data_disk_enable_index = user_data
        .find("systemctl enable --now ctx-avf-data-disk.service")
        .expect("data-disk enable command");
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
    assert!(
        data_disk_enable_index
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
fn materialize_writable_rootfs_image_preserves_small_rootfs_size() {
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
        1024 * 1024
    );
    let note = note.expect("copy note");
    assert!(note.contains("copied rootfs image"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_cloud_init_seed_digest_changes_when_payload_inputs_change() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-cloud-init-seed-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let guest_agent_bytes = b"guest-agent-v1";
    let meta_v1 = render_shared_vm_cloud_init_meta_data(&temp, guest_agent_bytes, None, "sha-one");
    let user_v1 = render_shared_vm_cloud_init_user_data(
        &temp,
        guest_agent_bytes,
        None,
        &temp.join("payloads").join("container-stack.tar.gz"),
        "sha-one",
    );
    let network = render_shared_vm_cloud_init_network_config();
    let digest_v1 = shared_vm_cloud_init_seed_digest(&meta_v1, &user_v1, &network);

    let meta_v2 = render_shared_vm_cloud_init_meta_data(&temp, guest_agent_bytes, None, "sha-two");
    let user_v2 = render_shared_vm_cloud_init_user_data(
        &temp,
        guest_agent_bytes,
        None,
        &temp.join("payloads").join("container-stack.tar.gz"),
        "sha-two",
    );
    let digest_v2 = shared_vm_cloud_init_seed_digest(&meta_v2, &user_v2, &network);

    assert_ne!(digest_v1, digest_v2);
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn default_shared_vm_kernel_cmdline_targets_runtime_rootfs_label() {
    let cmdline = default_shared_vm_kernel_cmdline();

    assert!(cmdline.contains("console=hvc0"));
    assert!(cmdline.contains("root=LABEL=cloudimg-rootfs"));
    assert!(cmdline.contains("rootwait"));
    assert!(cmdline.contains("rw"));
}

#[test]
fn materialize_data_disk_image_initializes_sparse_guest_data_disk() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-data-disk-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");

    let (data_disk, note) = materialize_data_disk_image(&temp).expect("materialize data disk");

    assert_eq!(data_disk, shared_vm_data_disk_path(&temp));
    assert_eq!(
        fs::metadata(&data_disk).expect("data-disk metadata").len(),
        SHARED_VM_INITIAL_DATA_DISK_BYTES
    );
    let note = note.expect("data-disk note");
    assert!(note.contains("initialized sparse AVF Linux data disk"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn resolve_shared_vm_data_disk_growth_decision_returns_no_action_when_guest_free_is_healthy() {
    let decision = resolve_shared_vm_data_disk_growth_decision(
        SHARED_VM_INITIAL_DATA_DISK_BYTES,
        SHARED_VM_DATA_DISK_GROWTH_THRESHOLD_BYTES,
        gibibytes(64),
    );

    assert_eq!(decision, SharedVmDataDiskGrowthDecision::NoAction);
}

#[test]
fn resolve_shared_vm_data_disk_growth_decision_grows_when_guest_free_is_low_and_host_has_budget() {
    let decision = resolve_shared_vm_data_disk_growth_decision(
        SHARED_VM_INITIAL_DATA_DISK_BYTES,
        gibibytes(1),
        gibibytes(64),
    );

    assert_eq!(
        decision,
        SharedVmDataDiskGrowthDecision::Grow {
            new_size_bytes: SHARED_VM_INITIAL_DATA_DISK_BYTES
                + SHARED_VM_DATA_DISK_GROWTH_STEP_BYTES,
            additional_bytes: SHARED_VM_DATA_DISK_GROWTH_STEP_BYTES,
        }
    );
}

#[test]
fn resolve_shared_vm_data_disk_growth_decision_blocks_when_host_reserve_would_be_breached() {
    let decision = resolve_shared_vm_data_disk_growth_decision(
        SHARED_VM_INITIAL_DATA_DISK_BYTES,
        gibibytes(1),
        SHARED_VM_HOST_DISK_RESERVE_BYTES,
    );

    assert_eq!(
        decision,
        SharedVmDataDiskGrowthDecision::HostReserveBlocked {
            available_host_bytes: SHARED_VM_HOST_DISK_RESERVE_BYTES,
            reserve_bytes: SHARED_VM_HOST_DISK_RESERVE_BYTES,
            requested_additional_bytes: SHARED_VM_DATA_DISK_GROWTH_STEP_BYTES,
        }
    );
}

#[test]
fn resolve_shared_vm_memory_balloon_action_reclaims_under_host_pressure() {
    let action = resolve_shared_vm_memory_balloon_action(
        gibibytes(16),
        gibibytes(16),
        gibibytes(4),
        Some(gibibytes(8)),
        gibibytes(3),
    );

    assert_eq!(
        action,
        SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes: gibibytes(14),
            available_host_bytes: gibibytes(3),
            aggressive: false,
        }
    );
}

#[test]
fn resolve_shared_vm_memory_balloon_action_grows_under_guest_pressure() {
    let action = resolve_shared_vm_memory_balloon_action(
        gibibytes(8),
        gibibytes(16),
        gibibytes(4),
        Some(gibibytes(1)),
        gibibytes(10),
    );

    assert_eq!(
        action,
        SharedVmMemoryBalloonAction::Grow {
            new_target_bytes: gibibytes(10),
            available_host_bytes: gibibytes(10),
            guest_available_bytes: gibibytes(1),
        }
    );
}

#[test]
fn resolve_shared_vm_memory_balloon_action_requests_emergency_stop_at_floor() {
    let action = resolve_shared_vm_memory_balloon_action(
        gibibytes(4),
        gibibytes(16),
        gibibytes(4),
        Some(gibibytes(1)),
        gibibytes(0),
    );

    assert_eq!(
        action,
        SharedVmMemoryBalloonAction::EmergencyStop {
            available_host_bytes: gibibytes(0),
            current_target_bytes: gibibytes(4),
            floor_bytes: gibibytes(4),
        }
    );
}

#[test]
fn resolve_shared_vm_memory_watchdog_sample_action_requires_confirmed_emergency_pressure() {
    let first = resolve_shared_vm_memory_watchdog_sample_action(0, gibibytes(0));
    assert_eq!(
        first,
        SharedVmMemoryWatchdogSampleAction::NoAction {
            next_consecutive_emergency_samples: 1,
        }
    );

    let second = resolve_shared_vm_memory_watchdog_sample_action(1, gibibytes(0));
    assert_eq!(
        second,
        SharedVmMemoryWatchdogSampleAction::RequestStop {
            next_consecutive_emergency_samples: 2,
            available_host_bytes: gibibytes(0),
        }
    );
}

#[test]
fn resolve_shared_vm_memory_watchdog_sample_action_resets_after_host_recovers() {
    let action = resolve_shared_vm_memory_watchdog_sample_action(1, gibibytes(2));
    assert_eq!(
        action,
        SharedVmMemoryWatchdogSampleAction::NoAction {
            next_consecutive_emergency_samples: 0,
        }
    );
}

#[test]
fn resolve_shared_vm_memory_watchdog_exit_action_models_sigterm_and_sigkill_escalation() {
    assert_eq!(
        resolve_shared_vm_memory_watchdog_exit_action(true, false),
        SharedVmMemoryWatchdogExitAction::OwnerExitedAfterRequest
    );
    assert_eq!(
        resolve_shared_vm_memory_watchdog_exit_action(false, true),
        SharedVmMemoryWatchdogExitAction::OwnerExitedAfterSigterm
    );
    assert_eq!(
        resolve_shared_vm_memory_watchdog_exit_action(false, false),
        SharedVmMemoryWatchdogExitAction::EscalateToSigkill
    );
}

#[cfg(target_os = "macos")]
#[test]
fn persist_shared_vm_owner_error_state_marks_vm_error_and_clears_owner_processes() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-owner-error-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let state_path = temp.join("shared-vm-state.json");
    let mut state = PersistedSharedVmState {
        state: AvfLinuxSharedVmLifecycleState::Running,
        runtime_root: Some(temp.join("runtime")),
        rootfs_image: Some(temp.join("rootfs.raw")),
        kernel_path: Some(temp.join("kernel")),
        initrd_path: Some(temp.join("initrd")),
        runtime_version: Some("test-runtime".to_string()),
        updated_at: None,
        last_started_at: Some("started".to_string()),
        last_saved_at: Some("saved".to_string()),
        last_stopped_at: None,
        transition_status: Some(AvfLinuxSharedVmTransitionStatus::Scaffolded),
        relay_pid: Some(101),
        guest_agent_pid: Some(202),
        simulated: false,
        notes: vec!["running".to_string()],
    };

    persist_shared_vm_owner_error_state(
        &state_path,
        &mut state,
        "disk growth blocked by host reserve".to_string(),
    )
    .expect("persist owner error state");

    assert!(matches!(state.state, AvfLinuxSharedVmLifecycleState::Error));
    assert!(!state.simulated);
    assert!(state.updated_at.is_some());
    assert_eq!(state.last_stopped_at, state.updated_at);
    assert!(state.transition_status.is_none());
    assert!(state.relay_pid.is_none());
    assert!(state.guest_agent_pid.is_none());
    assert_eq!(
        state.notes,
        vec!["disk growth blocked by host reserve".to_string()]
    );

    let persisted = load_state(&state_path)
        .expect("load state")
        .expect("persisted state");
    assert!(matches!(
        persisted.state,
        AvfLinuxSharedVmLifecycleState::Error
    ));
    assert!(persisted.relay_pid.is_none());
    assert!(persisted.guest_agent_pid.is_none());
    assert_eq!(
        persisted.notes,
        vec!["disk growth blocked by host reserve".to_string()]
    );
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_state_marks_missing_owner_with_memory_pressure_request_as_error() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-memory-request-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    prepare_runtime_layout(&temp).expect("prepare runtime layout");
    let state_path = shared_vm_state_path(&temp);
    persist_state(
        &state_path,
        &PersistedSharedVmState {
            state: AvfLinuxSharedVmLifecycleState::Running,
            runtime_root: None,
            rootfs_image: None,
            kernel_path: None,
            initrd_path: None,
            runtime_version: None,
            updated_at: Some(now_timestamp_string()),
            last_started_at: None,
            last_saved_at: None,
            last_stopped_at: None,
            transition_status: Some(AvfLinuxSharedVmTransitionStatus::Scaffolded),
            relay_pid: Some(999_999),
            guest_agent_pid: None,
            simulated: false,
            notes: vec!["running".to_string()],
        },
    )
    .expect("persist running state");
    request_shared_vm_memory_pressure_stop(&temp, "watchdog requested an emergency stop")
        .expect("request emergency stop");

    let response = shared_vm_state(&temp).expect("shared vm state");

    assert!(matches!(
        response.state,
        AvfLinuxSharedVmLifecycleState::Error
    ));
    assert!(response.transition_status.is_none());
    assert!(response
        .notes
        .iter()
        .any(|note| note.contains("watchdog requested an emergency stop")));
    assert!(
        !shared_vm_memory_pressure_request_path(&temp).exists(),
        "request file should be cleared once state is updated"
    );
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn start_shared_vm_materializes_rootfs_and_data_disk_layout() {
    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-start-layout-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    let runtime_root = temp.join("runtime");
    let helpers_root = runtime_root.join("helpers");
    fs::create_dir_all(&helpers_root).expect("create helpers root");
    let source_rootfs = temp.join("source-rootfs.raw");
    fs::write(&source_rootfs, b"rootfs").expect("write rootfs");
    let kernel_path = helpers_root.join("kernel");
    fs::write(&kernel_path, b"kernel").expect("write kernel");
    let initrd_path = helpers_root.join("initrd");
    fs::write(&initrd_path, b"initrd").expect("write initrd");

    let started = start_shared_vm(
        &temp,
        &runtime_root,
        &source_rootfs,
        &kernel_path,
        &initrd_path,
        "test-runtime".to_string(),
    )
    .expect("start shared vm");

    assert!(matches!(
        started.state,
        AvfLinuxSharedVmLifecycleState::Running
    ));
    assert!(started.simulated);
    assert!(shared_vm_rootfs_path(&temp).exists());
    assert!(shared_vm_data_disk_path(&temp).exists());
    assert!(started
        .notes
        .iter()
        .any(|note| note.contains("AVF Linux data disk")));
    assert!(started
        .notes
        .iter()
        .any(|note| note.contains("shared VM start reached launch-ready")));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_guest_readiness_args_include_bridge_probe() {
    let rendered = shared_vm_guest_readiness_args().join(" ");
    assert!(rendered.contains("bridge_probe_failed"));
    assert!(rendered.contains("readiness phase"));
    assert!(rendered.contains("ip link add name \"$probe_bridge\" type bridge"));
    assert!(rendered.contains(SHARED_VM_GUEST_NERDCTL_BIN));
    assert!(rendered.contains(SHARED_VM_GUEST_BUILDKITCTL_BIN));
    assert!(rendered.contains(&format!(
        "timeout --kill-after=1s --preserve-status {SHARED_VM_READINESS_PHASE_TIMEOUT_SECONDS}s"
    )));
}

#[test]
fn readiness_phase_line_extraction_filters_non_phase_output() {
    let stdout = b"hello\n[ctx-avf-linux] readiness phase buildctl ok in 12ms\n";
    let stderr =
        b"noise\n[ctx-avf-linux] readiness phase bridge-probe failed with exit 41 after 3ms\n";
    let lines = extract_shared_vm_readiness_phase_lines(stdout, stderr);
    assert_eq!(
        lines,
        vec![
            "[ctx-avf-linux] readiness phase buildctl ok in 12ms".to_string(),
            "[ctx-avf-linux] readiness phase bridge-probe failed with exit 41 after 3ms"
                .to_string(),
        ]
    );
}

#[test]
fn readiness_phase_summary_strips_helper_prefix() {
    let summary = summarize_shared_vm_readiness_phase_lines(&[
        "[ctx-avf-linux] readiness phase containerd ok in 10ms".to_string(),
        "[ctx-avf-linux] readiness phase buildkit ok in 11ms".to_string(),
    ]);
    assert_eq!(summary, "containerd ok in 10ms, buildkit ok in 11ms");
}

#[test]
fn cold_boot_timeout_extends_when_rootfs_is_materialized() {
    assert_eq!(
        default_real_guest_exec_ready_timeout(),
        Duration::from_secs(30)
    );
    assert_eq!(
        cold_boot_real_guest_exec_ready_timeout(),
        Duration::from_secs(600)
    );
    assert_eq!(
        real_guest_exec_ready_timeout_for_start(None, true),
        default_real_guest_exec_ready_timeout()
    );
    assert_eq!(
        real_guest_exec_ready_timeout_for_start(
            Some("copied rootfs image into helper-managed writable path"),
            true
        ),
        cold_boot_real_guest_exec_ready_timeout()
    );
    assert_eq!(
        real_guest_exec_ready_timeout_for_start(None, false),
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
    let data_disk_enable_index = user_data
        .find("systemctl enable --now ctx-avf-data-disk.service")
        .expect("data-disk enable command");
    let prepare_guest_agent_index = user_data
        .find("preparing ctx-avf-linux-guest-agent.service")
        .expect("prepare guest-agent command");
    assert!(daemon_reload_index < host_data_enable_index);
    assert!(host_data_enable_index < data_disk_enable_index);
    assert!(data_disk_enable_index < prepare_guest_agent_index);
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
    let ready_marker = shared_vm_guest_control_ready_path(&temp);
    let saved_state = shared_vm_saved_state_path(&temp);
    let rootfs = shared_vm_rootfs_path(&temp);
    let data_disk = shared_vm_data_disk_path(&temp);
    for path in [
        &control_socket,
        &guest_agent_socket,
        &ready_marker,
        &saved_state,
        &rootfs,
    ] {
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, b"x").expect("seed file");
    }
    fs::create_dir_all(data_disk.parent().expect("data-disk parent")).expect("create parent");
    fs::write(&data_disk, b"x").expect("seed data-disk");

    reset_writable_shared_vm_runtime_state(&temp).expect("reset runtime state");

    for path in [
        &control_socket,
        &guest_agent_socket,
        &ready_marker,
        &saved_state,
        &rootfs,
    ] {
        assert!(!path.exists(), "{} should be removed", path.display());
    }
    assert!(
        data_disk.exists(),
        "{} should be preserved",
        data_disk.display()
    );
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_start_lock_times_out_while_live_holder_exists() {
    let temp = std::env::temp_dir().join(format!(
        "ctx-avf-start-lock-timeout-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(shared_vm_root(&temp)).expect("create shared vm root");
    fs::write(
        shared_vm_start_lock_path(&temp),
        format!("{}\n", std::process::id()),
    )
    .expect("seed live start lock");

    let err = acquire_shared_vm_start_lock(&temp, Duration::from_millis(100))
        .expect_err("live holder should block acquisition");
    assert!(err
        .to_string()
        .contains("timed out waiting for shared VM start lock"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_start_lock_replaces_stale_holder() {
    let temp = std::env::temp_dir().join(format!(
        "ctx-avf-start-lock-stale-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(shared_vm_root(&temp)).expect("create shared vm root");
    fs::write(shared_vm_start_lock_path(&temp), "999999\n").expect("seed stale start lock");

    let guard = acquire_shared_vm_start_lock(&temp, Duration::from_secs(1))
        .expect("stale holder should be replaced");
    let raw = fs::read_to_string(shared_vm_start_lock_path(&temp)).expect("read lock");
    assert_eq!(
        parse_shared_vm_start_lock_pid(&raw),
        Some(std::process::id())
    );
    drop(guard);
    assert!(
        !shared_vm_start_lock_path(&temp).exists(),
        "lock should be removed when guard drops"
    );
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn cloud_init_meta_data_changes_when_guest_payload_changes() {
    let first = render_shared_vm_cloud_init_meta_data(
        Path::new("/tmp/a"),
        b"guest-agent-a",
        Some(b"egress-proxy-a"),
        "container-stack-a",
    );
    let second = render_shared_vm_cloud_init_meta_data(
        Path::new("/tmp/b"),
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

#[test]
fn wait_for_guest_control_ready_marker_observes_marker_creation() {
    let temp = std::env::temp_dir().join(format!(
        "ctx-avf-ready-marker-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let marker = shared_vm_guest_control_ready_path(&temp);
    let marker_for_thread = marker.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        if let Some(parent) = marker_for_thread.parent() {
            fs::create_dir_all(parent).expect("create marker parent");
        }
        fs::write(&marker_for_thread, b"ready").expect("write ready marker");
    });

    wait_for_guest_control_ready_marker(&temp, Duration::from_secs(1))
        .expect("marker should become ready");
    writer.join().expect("writer thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn wait_for_guest_control_ready_marker_times_out_without_marker() {
    let temp = std::env::temp_dir().join(format!(
        "ctx-avf-ready-marker-timeout-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");

    let err = wait_for_guest_control_ready_marker(&temp, Duration::from_millis(100))
        .expect_err("missing marker should time out");
    assert!(err
        .to_string()
        .contains("timed out waiting for guest control ready marker"));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[test]
fn shared_vm_owner_guest_probe_ready_requires_guest_control_marker() {
    let temp = std::env::temp_dir().join(format!(
        "ctx-avf-owner-probe-ready-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    assert!(!shared_vm_owner_guest_probe_ready(&temp));
    let marker = shared_vm_guest_control_ready_path(&temp);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).expect("create marker parent");
    }
    fs::write(&marker, b"ready").expect("write ready marker");
    assert!(shared_vm_owner_guest_probe_ready(&temp));
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(all(target_os = "macos", unix))]
#[test]
fn wait_for_real_guest_exec_ready_succeeds_without_guest_control_marker() {
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctx-avf-real-ready-no-marker-{}-{}",
        std::process::id(),
        now_timestamp_string()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).expect("clear tempdir");
    }
    fs::create_dir_all(&temp).expect("create tempdir");
    let listener = bind_shared_vm_control_listener(&temp).expect("bind control socket");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept control socket");
        let frame = read_exec_frame(&mut stream)
            .expect("read request")
            .expect("request frame");
        let request = match frame {
            AvfLinuxExecFrame::Request(request) => request,
            other => panic!("expected request frame, got {other:?}"),
        };
        assert_eq!(request.command, "/bin/sh");
        assert_eq!(request.cwd, "/");
        assert_eq!(request.user.as_deref(), Some("root"));
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stderr(
                b"[ctx-avf-linux] readiness phase containerd ok in 10ms\n[ctx-avf-linux] readiness phase buildkit ok in 11ms\n"
                    .to_vec(),
            ),
        )
        .expect("write readiness phases");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 }),
        )
        .expect("write exit frame");
    });

    let readiness =
        wait_for_real_guest_exec_ready(&temp, Duration::from_secs(1)).expect("guest ready");
    assert_eq!(readiness.attempts, 1);
    assert_eq!(
        readiness.phase_lines,
        vec![
            "[ctx-avf-linux] readiness phase containerd ok in 10ms".to_string(),
            "[ctx-avf-linux] readiness phase buildkit ok in 11ms".to_string(),
        ]
    );
    assert!(!shared_vm_owner_guest_probe_ready(&temp));

    server.join().expect("server thread");
    let control_socket = shared_vm_control_socket_path(&temp);
    if control_socket.exists() {
        fs::remove_file(&control_socket).expect("cleanup control socket");
    }
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
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
fn guest_exec_capture_with_socket_timeout_fails_when_server_never_responds() {
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-capture-timeout-{}-{}",
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
        let (_stream, _) = listener.accept().expect("accept control socket");
        thread::sleep(Duration::from_millis(300));
    });

    let result = run_guest_exec_capture_with_socket_timeout(
        &socket_path,
        Path::new("/"),
        "/usr/bin/true",
        &[],
        Some("root"),
        HashMap::new(),
        None,
        Some(Duration::from_millis(100)),
    );
    let err = match result {
        Ok(_) => panic!("capture should time out when the server never responds"),
        Err(err) => err,
    };

    let rendered = format!("{err:#}");
    assert!(
        rendered.contains("timed out")
            || rendered.contains("deadline")
            || rendered.contains("Resource temporarily unavailable")
            || rendered.contains("operation would block")
    );

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
fn shared_vm_relay_restores_blocking_mode_for_nonblocking_clients() {
    use std::os::fd::{FromRawFd, IntoRawFd};
    use std::thread;
    use std::time::Duration;

    let (mut client, relay_client) = UnixStream::pair().expect("client pair");
    relay_client
        .set_nonblocking(true)
        .expect("set relay client nonblocking");
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

        let payload = vec![b'x'; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
        for _ in 0..2048 {
            write_exec_frame(
                &mut guest_server,
                &AvfLinuxExecFrame::Stdout(payload.clone()),
            )
            .expect("write stdout frame burst");
        }
        write_exec_frame(
            &mut guest_server,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 }),
        )
        .expect("write exit frame");
    });

    thread::sleep(Duration::from_millis(100));

    let mut received = 0usize;
    loop {
        let frame = read_exec_frame(&mut client)
            .expect("read relayed frame")
            .expect("relayed frame");
        match frame {
            AvfLinuxExecFrame::Stdout(bytes) => received += bytes.len(),
            AvfLinuxExecFrame::Exit(exit) => {
                assert_eq!(exit.exit_code, 0);
                break;
            }
            other => panic!("unexpected relayed frame: {other:?}"),
        }
    }

    assert_eq!(received, 2048 * AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD);
    relay
        .join()
        .expect("relay thread join")
        .expect("relay should succeed once client starts reading");
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
fn non_pty_guest_exec_cli_streams_output_before_stdin_eof() {
    use std::io::Read;
    use std::os::unix::net::UnixListener;
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::thread;
    use std::time::Duration;

    #[derive(Default)]
    struct ReaderState {
        sent_payload: bool,
        allow_eof: bool,
    }

    struct GateReader {
        state: Arc<(Mutex<ReaderState>, Condvar)>,
    }

    impl Read for GateReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let (lock, cv) = &*self.state;
            let mut state = lock.lock().expect("lock gated reader state");
            if !state.sent_payload {
                let payload = b"session.open\n";
                buf[..payload.len()].copy_from_slice(payload);
                state.sent_payload = true;
                return Ok(payload.len());
            }
            while !state.allow_eof {
                state = cv.wait(state).expect("wait for EOF gate");
            }
            Ok(0)
        }
    }

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-cli-streaming-{}-{}",
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
        assert_eq!(request.command, "/usr/bin/codex-crp");
        assert!(!request.pty);

        let stdin = read_exec_frame(&mut stream)
            .expect("read stdin frame")
            .expect("stdin frame");
        let AvfLinuxExecFrame::Stdin(stdin) = stdin else {
            panic!("expected stdin frame");
        };
        assert_eq!(stdin, b"session.open\n".to_vec());

        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stdout(b"session.opened\n".to_vec()),
        )
        .expect("write stdout frame");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 }),
        )
        .expect("write exit frame");
    });

    let gate = Arc::new((Mutex::new(ReaderState::default()), Condvar::new()));
    let reader = GateReader {
        state: Arc::clone(&gate),
    };
    let (result_tx, result_rx) = mpsc::channel();
    let socket_path_for_client = socket_path.clone();
    let worker = thread::spawn(move || {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run_guest_exec_cli_with_streaming_stdin(
            &socket_path_for_client,
            Path::new("/"),
            "/usr/bin/codex-crp",
            &[],
            None,
            HashMap::new(),
            Some(Box::new(reader)),
            &mut stdout,
            &mut stderr,
        );
        result_tx
            .send((result, stdout, stderr))
            .expect("send streaming exec result");
    });

    let (result, stdout, stderr) = match result_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(err) => {
            let (lock, cv) = &*gate;
            let mut state = lock.lock().expect("lock EOF gate after timeout");
            state.allow_eof = true;
            cv.notify_all();
            panic!("timed out waiting for streaming exec result: {err}");
        }
    };

    let (lock, cv) = &*gate;
    let mut state = lock.lock().expect("lock EOF gate");
    state.allow_eof = true;
    cv.notify_all();
    drop(state);

    worker.join().expect("streaming exec worker");
    assert_eq!(result.expect("streaming exec should succeed"), 0);
    assert_eq!(
        String::from_utf8(stdout).expect("stdout utf8"),
        "session.opened\n"
    );
    assert!(stderr.is_empty());
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
fn guest_exec_capture_returns_exit_and_stderr_when_guest_exits_early_during_streamed_stdin() {
    use std::io::Cursor;
    use std::os::unix::net::UnixListener;
    use std::thread;

    let temp = PathBuf::from("/tmp").join(format!(
        "ctxavf-cli-early-exit-{}-{}",
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

        let stdin = read_exec_frame(&mut stream)
            .expect("read first stdin frame")
            .expect("stdin frame");
        assert!(matches!(stdin, AvfLinuxExecFrame::Stdin(_)));

        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Stderr(b"tar: Unexpected EOF in archive\n".to_vec()),
        )
        .expect("write stderr frame");
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 2 }),
        )
        .expect("write exit frame");
    });

    let mut stdin = Cursor::new(vec![b'x'; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD * 4]);
    let result = run_guest_exec_capture(
        &socket_path,
        Path::new("/"),
        "/usr/bin/tar",
        &["-xpf".to_string(), "-".to_string()],
        None,
        HashMap::new(),
        Some(&mut stdin),
    )
    .expect("capture path should surface the guest exit instead of a broken pipe");

    assert_eq!(result.exit_code, 2);
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).expect("stderr utf8"),
        "tar: Unexpected EOF in archive\n"
    );

    server.join().expect("server thread");
    fs::remove_dir_all(&temp).expect("cleanup tempdir");
}

#[cfg(unix)]
#[test]
fn guest_exec_capture_over_connected_stream_returns_output_without_reentering_relay() {
    use std::thread;

    let (mut client, mut server) = UnixStream::pair().expect("unix stream pair");
    let guest = thread::spawn(move || {
        let request = read_exec_frame(&mut server)
            .expect("read request")
            .expect("request frame");
        let request = match request {
            AvfLinuxExecFrame::Request(request) => request,
            other => panic!("expected request frame, got {other:?}"),
        };
        assert_eq!(request.command, "/bin/sh");
        assert_eq!(request.cwd, "/");

        let close = read_exec_frame(&mut server)
            .expect("read close stdin")
            .expect("close stdin frame");
        assert!(matches!(close, AvfLinuxExecFrame::CloseStdin));

        write_exec_frame(&mut server, &AvfLinuxExecFrame::Stdout(b"12345\n".to_vec()))
            .expect("write stdout");
        write_exec_frame(&mut server, &AvfLinuxExecFrame::Stderr(b"warn\n".to_vec()))
            .expect("write stderr");
        write_exec_frame(
            &mut server,
            &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 }),
        )
        .expect("write exit");
    });

    let result = run_guest_exec_capture_over_connected_stream(
        &mut client,
        Path::new("/"),
        "/bin/sh",
        &["-lc".to_string(), "echo 12345".to_string()],
        Some("root"),
        HashMap::new(),
    )
    .expect("capture over connected stream should succeed");

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8(result.stdout).expect("stdout utf8"),
        "12345\n"
    );
    assert_eq!(
        String::from_utf8(result.stderr).expect("stderr utf8"),
        "warn\n"
    );
    guest.join().expect("guest thread");
}

#[cfg(unix)]
#[test]
fn exec_stream_payload_budget_stays_within_shared_vm_safe_limit() {
    const {
        assert!(
            AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD <= 1024,
            "shared-VM exec transport truncated larger stdin frames in live tar-import repros",
        );
    }
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

#[cfg(unix)]
#[test]
fn guest_agent_exec_emits_exit_without_waiting_for_close_stdin() {
    use std::os::unix::net::UnixStream;
    use std::thread;
    use std::time::Duration;

    let (mut client, server) = UnixStream::pair().expect("unix stream pair");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set client read timeout");
    let relay = thread::spawn(move || {
        handle_guest_agent_control_connection(Path::new("/tmp"), server)
            .expect("handle guest-agent control connection");
    });

    write_exec_frame(
        &mut client,
        &AvfLinuxExecFrame::Request(AvfLinuxExecRequest::new(
            "/bin/sh",
            vec!["-lc".to_string(), "printf ready".to_string()],
            "/",
            None,
            HashMap::new(),
            false,
        )),
    )
    .expect("write client request");

    let stdout = read_exec_frame(&mut client)
        .expect("read stdout frame")
        .expect("stdout frame");
    let stdout = match stdout {
        AvfLinuxExecFrame::Stdout(bytes) => bytes,
        other => panic!("expected stdout frame, got {other:?}"),
    };
    assert_eq!(stdout, b"ready".to_vec());

    let exit = read_exec_frame(&mut client)
        .expect("read exit frame")
        .expect("exit frame");
    assert_eq!(
        exit,
        AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 0 })
    );

    relay.join().expect("guest-agent relay thread");
}
