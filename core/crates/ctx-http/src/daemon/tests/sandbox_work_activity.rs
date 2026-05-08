use super::*;

#[tokio::test]
async fn sandbox_work_activity_ignores_host_turns() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Host,
        SessionTurnStatus::Running,
    )
    .await;

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(!activity.active);
    assert_eq!(activity.active_sandbox_turn_count, 0);
    assert_eq!(activity.running_sandbox_turn_count, 0);
    assert!(activity.turns.is_empty());
}

#[tokio::test]
async fn sandbox_work_activity_counts_sandbox_turns() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Sandbox,
        SessionTurnStatus::Queued,
    )
    .await;
    let _ = create_session_with_turn_status(
        &state,
        temp.path(),
        ExecutionEnvironment::Sandbox,
        SessionTurnStatus::Running,
    )
    .await;

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.active_sandbox_turn_count, 2);
    assert_eq!(activity.queued_sandbox_turn_count, 1);
    assert_eq!(activity.running_sandbox_turn_count, 1);
    assert_eq!(activity.turns.len(), 2);
}

#[tokio::test]
async fn sandbox_work_activity_counts_runtime_operations() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _runtime_guard = state.execution.harness.begin_runtime_operation();

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.runtime_operation_count, 1);
}

#[tokio::test]
async fn sandbox_work_activity_counts_prewarm_operations() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let _prewarm_guard = state.execution.harness.begin_prewarm_artifact_activity();

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.prewarm_artifact_operation_count, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_work_activity_counts_container_backed_terminals() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));
    let cli_path = temp.path().join("container-terminal-cli.sh");
    std::fs::write(&cli_path, "#!/bin/sh\ntrap 'exit 0' TERM INT\nsleep 60\n").unwrap();
    std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let terminal = state
        .transport
        .terminals
        .create(ctx_transport_runtime::terminals::TerminalCreateRequest {
            workspace_id: WorkspaceId::new(),
            task_id: Some(TaskId::new()),
            session_id: None,
            worktree_id: Some(WorktreeId::new()),
            cwd: temp.path().to_path_buf(),
            shell: "/bin/sh".to_string(),
            cols: None,
            rows: None,
            env: HashMap::new(),
            native_container: Some(
                ctx_transport_runtime::terminals::NativeContainerTerminalSpec {
                    cli_bin: cli_path,
                    cli_env: HashMap::new(),
                    container_name: "ctx-harness-terminal".to_string(),
                    workdir: "/workspace".to_string(),
                    user: None,
                },
            ),
            shared_vm_container: None,
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let activity = loop {
        let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
        if activity.active && activity.running_container_backed_terminal {
            break activity;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for container-backed terminal activity: {activity:#?}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(activity.active);
    assert!(activity.running_container_backed_terminal);

    let _ = terminal.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_work_activity_counts_running_workspace_containers() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let _disable = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let temp = tempdir().unwrap();
    let cli_path = temp.path().join("sandbox-cli.sh");
    let log_path = temp.path().join("sandbox-cli.log");
    std::fs::write(
        &cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"ls\" ] && [ \"$3\" = \"--format\" ] && [ \"$4\" = \"{{{{.Names}}}}\" ]; then\n  printf 'ctx-harness-one\\nctx-harness-two\\npostgres\\n'\n  exit 0\nfi\necho \"unexpected invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _guard = EnvVarGuard::set(
        CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &cli_path.to_string_lossy(),
    );
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://localhost".to_string(),
        None,
    ));

    let activity = daemon_sandbox_work_activity_summary(&state).await.unwrap();
    assert!(activity.active);
    assert_eq!(activity.running_workspace_container_count, 2);
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains("container ls --format {{.Names}}"),
        "expected running-container probe in log:\n{log}"
    );
}
