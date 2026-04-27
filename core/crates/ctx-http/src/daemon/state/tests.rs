use super::*;
use ctx_execution_runtime::StartupPrewarmState;

async fn test_state(temp: &tempfile::TempDir) -> Arc<AppState> {
    Arc::new(AppState::new(
        temp.path().to_path_buf(),
        StoreManager::open(temp.path()).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

#[tokio::test]
async fn test_state_does_not_auto_spawn_startup_prewarm_in_unit_tests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(&temp).await;

    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let startup = state.execution.setup.startup_status().await;
    assert_eq!(startup.state, StartupPrewarmState::Idle);
    assert!(startup.last_attempt_at.is_none(), "{startup:?}");
}

#[tokio::test]
async fn find_running_install_reconciles_stale_running_venv_install() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(&temp).await;
    let install_id = InstallId::new_v4();
    let now = chrono::Utc::now();
    let mut install = InstallState::new("mistral".to_string(), Some(InstallTarget::Container));
    install.started_at = now - chrono::Duration::minutes(9);
    install.events.push_back(InstallProgressEvent {
        install_id,
        provider_id: "mistral".to_string(),
        target: Some(InstallTarget::Container),
        at: now - chrono::Duration::minutes(8),
        stage: "venv".to_string(),
        message: "Creating virtualenv…".to_string(),
        level: InstallEventLevel::Info,
        bytes: None,
        total_bytes: None,
        attempt: None,
        error_code: None,
    });
    state
        .providers
        .installs
        .lock()
        .await
        .insert(install_id, install);

    let running = state
        .find_running_install("mistral", Some(InstallTarget::Container))
        .await;
    assert!(running.is_none());

    let info = state
        .get_install_info(install_id)
        .await
        .expect("missing install info");
    assert!(matches!(info.state, InstallStateKind::Failed));
    assert_eq!(info.error_code, Some(InstallErrorCode::Timeout));
    assert!(info
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("timed out during venv"));
}

#[tokio::test]
async fn get_install_info_preserves_recent_running_install() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(&temp).await;
    let install_id = InstallId::new_v4();
    let now = chrono::Utc::now();
    let mut install = InstallState::new("codex".to_string(), Some(InstallTarget::Container));
    install.started_at = now - chrono::Duration::minutes(1);
    install.events.push_back(InstallProgressEvent {
        install_id,
        provider_id: "codex".to_string(),
        target: Some(InstallTarget::Container),
        at: now - chrono::Duration::seconds(30),
        stage: "download".to_string(),
        message: "downloading…".to_string(),
        level: InstallEventLevel::Info,
        bytes: Some(10),
        total_bytes: Some(100),
        attempt: None,
        error_code: None,
    });
    state
        .providers
        .installs
        .lock()
        .await
        .insert(install_id, install);

    let info = state
        .get_install_info(install_id)
        .await
        .expect("missing install info");
    assert!(matches!(info.state, InstallStateKind::Running));
    assert_eq!(info.error_code, None);
}

#[tokio::test]
async fn start_install_dedupes_concurrent_requests_for_same_provider_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(&temp).await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            state
                .start_install("acp-crp-bridge".to_string(), Some(InstallTarget::Container))
                .await
        }));
    }

    let mut install_ids = Vec::new();
    let mut started_new_count = 0usize;
    for task in tasks {
        let (install_id, started_new) = task.await.expect("join start_install task");
        install_ids.push(install_id);
        if started_new {
            started_new_count += 1;
        }
    }

    assert_eq!(
        started_new_count, 1,
        "concurrent start_install callers must share one tracked running install"
    );
    assert!(
        install_ids
            .windows(2)
            .all(|pair| pair.first() == pair.get(1)),
        "all concurrent start_install callers should receive the same install id: {install_ids:#?}"
    );
}

#[tokio::test]
async fn mirrored_prerequisite_progress_does_not_inherit_source_byte_completion() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(&temp).await;

    let (source_install_id, _) = state
        .start_install("acp-crp-bridge".to_string(), Some(InstallTarget::Container))
        .await;
    let (mirror_install_id, _) = state
        .start_install("cursor".to_string(), Some(InstallTarget::Container))
        .await;

    assert!(
        state
            .register_install_progress_mirror(source_install_id, mirror_install_id)
            .await
    );

    state
        .emit_install_event(
            source_install_id,
            InstallProgressEvent {
                install_id: source_install_id,
                provider_id: "acp-crp-bridge".to_string(),
                target: Some(InstallTarget::Container),
                at: chrono::Utc::now(),
                stage: "download".to_string(),
                message: "downloading…".to_string(),
                level: InstallEventLevel::Info,
                bytes: Some(10),
                total_bytes: Some(10),
                attempt: Some(1),
                error_code: None,
            },
        )
        .await;

    let mirror_info = state
        .get_install_polling_info(mirror_install_id)
        .await
        .expect("mirror install info");

    assert!(matches!(mirror_info.state, InstallStateKind::Running));
    assert_eq!(mirror_info.progress_pct, Some(2));
    assert_eq!(
        mirror_info
            .last_event
            .as_ref()
            .map(|event| event.stage.as_str()),
        Some("start")
    );
    assert_eq!(
        mirror_info
            .last_event
            .as_ref()
            .and_then(|event| event.bytes),
        None
    );
    assert_eq!(
        mirror_info
            .last_event
            .as_ref()
            .map(|event| event.message.contains("Prerequisite acp-crp-bridge")),
        Some(true)
    );
}
