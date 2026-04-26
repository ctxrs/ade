use super::*;
use crate::title_generation_local;
use std::collections::HashMap;

use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

async fn setup_state() -> (tempfile::TempDir, Arc<AppState>, Session) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let workspace = stores
        .global()
        .create_workspace(
            "ws".to_string(),
            data_dir.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = stores.workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            data_dir.path().to_string_lossy().to_string(),
            "base".to_string(),
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .unwrap();
    let task = store
        .create_task(
            workspace.id,
            title_generation::DEFAULT_SESSION_TITLE.to_string(),
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "fake-model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    stores
        .global()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));

    (data_dir, state, session)
}

async fn block_workspace_store_for_session(
    data_dir: &tempfile::TempDir,
    state: &Arc<AppState>,
    session: &Session,
) {
    state.cleanup_session(session.id).await;
    state
        .core
        .stores
        .evict_workspace(session.workspace_id)
        .await;

    let blocked_workspace_store_dir = data_dir
        .path()
        .join("db")
        .join("workspaces")
        .join(session.workspace_id.0.to_string());
    if let Ok(metadata) = tokio::fs::metadata(&blocked_workspace_store_dir).await {
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&blocked_workspace_store_dir)
                .await
                .unwrap();
        } else {
            tokio::fs::remove_file(&blocked_workspace_store_dir)
                .await
                .unwrap();
        }
    }
    tokio::fs::create_dir_all(blocked_workspace_store_dir.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&blocked_workspace_store_dir, b"blocked workspace store")
        .await
        .unwrap();
}

#[tokio::test]
async fn schedule_title_generation_falls_back_without_config() {
    let (_data_dir, state, session) = setup_state().await;
    let prompt = "make the title this: hello world";
    let spawned = schedule_session_title_generation(
        state.clone(),
        session.clone(),
        prompt.to_string(),
        false,
    )
    .await;

    assert!(!spawned);

    let store = state.store_for_session(session.id).await.unwrap();
    let updated = store.get_session(session.id).await.unwrap().unwrap();
    let expected = title_generation::fallback_title_from_prompt(prompt);
    assert_eq!(updated.title, expected);
}

#[tokio::test]
async fn generate_title_falls_back_when_local_runtime_missing() {
    let data_dir = tempfile::tempdir().unwrap();
    let model_path = title_generation_local::model_path(data_dir.path());
    if let Some(parent) = model_path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(&model_path, b"stub").await.unwrap();

    let cfg = user_settings::TitleGenerationSettings {
        mode: user_settings::TitleGenerationMode::Local,
        local: user_settings::TitleGenerationLocalSettings {
            model_id: title_generation_local::LOCAL_MODEL_ID.to_string(),
            use_json: true,
        },
        ..Default::default()
    };

    let prompt = "make the title this: hello world";
    let outcome = generate_title_for_prompt(Some(&cfg), prompt, data_dir.path())
        .await
        .unwrap();

    assert!(matches!(outcome.source, TitleGenerationSource::Fallback));
    assert_eq!(
        outcome.title,
        title_generation::fallback_title_from_prompt(prompt)
    );
}

#[tokio::test]
async fn existing_session_store_helper_returns_500_when_workspace_store_cannot_open() {
    let (data_dir, state, session) = setup_state().await;
    block_workspace_store_for_session(&data_dir, &state, &session).await;

    let result = store_for_existing_session_status(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected store open failure"),
        Err(status) => assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[tokio::test]
async fn existing_session_store_api_error_helper_returns_500_when_workspace_store_cannot_open() {
    let (data_dir, state, session) = setup_state().await;
    block_workspace_store_for_session(&data_dir, &state, &session).await;

    let result = store_for_existing_session_api_error(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected store open failure"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(!body.0.error.is_empty());
        }
    }
}

#[tokio::test]
async fn existing_session_write_store_helper_returns_500_when_workspace_store_cannot_open() {
    let (data_dir, state, session) = setup_state().await;
    block_workspace_store_for_session(&data_dir, &state, &session).await;

    let result = store_for_existing_session_status_for_write(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected store open failure"),
        Err(status) => assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[tokio::test]
async fn existing_session_write_store_api_error_helper_returns_500_when_workspace_store_cannot_open(
) {
    let (data_dir, state, session) = setup_state().await;
    block_workspace_store_for_session(&data_dir, &state, &session).await;

    let result = store_for_existing_session_api_error_for_write(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected store open failure"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(!body.0.error.is_empty());
        }
    }
}

#[tokio::test]
async fn existing_session_store_helpers_return_404_while_workspace_is_deleting() {
    let (_data_dir, state, session) = setup_state().await;
    state
        .core
        .stores
        .begin_workspace_delete(session.workspace_id)
        .await;

    let result = store_for_existing_session_status(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected delete-in-progress session to look missing"),
        Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
    }

    let result = store_for_existing_session_api_error(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected delete-in-progress session to look missing"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body.0.error, "session not found");
        }
    }

    let result = store_for_existing_session_status_for_write(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected delete-in-progress session to look missing"),
        Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
    }

    let result = store_for_existing_session_api_error_for_write(&state, session.id).await;
    match result {
        Ok(_) => panic!("expected delete-in-progress session to look missing"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body.0.error, "session not found");
        }
    }

    state
        .core
        .stores
        .finish_workspace_delete(session.workspace_id)
        .await;
}

#[tokio::test]
async fn existing_session_read_store_helpers_reject_archived_subagents_by_default() {
    let (_data_dir, state, session) = setup_state().await;
    let store = state
        .store_for_workspace(session.workspace_id)
        .await
        .unwrap();
    let child = store
        .create_session(
            session.task_id,
            session.workspace_id,
            session.worktree_id,
            session.execution_environment,
            "fake".to_string(),
            "fake-model".to_string(),
            "subagent".to_string(),
            Some(session.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(child.id, session.workspace_id)
        .await
        .unwrap();
    assert!(store
        .archive_subagent_session(session.id, child.id)
        .await
        .unwrap());

    match store_for_existing_session_status(&state, child.id).await {
        Ok(_) => panic!("archived child should be hidden from generic live read helpers"),
        Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
    }
    match store_for_existing_session_api_error(&state, child.id).await {
        Ok(_) => panic!("archived child should be hidden from generic API read helpers"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body.0.error, "session not found");
        }
    }
}

#[tokio::test]
async fn archived_history_store_helpers_allow_archived_subagents() {
    let (_data_dir, state, session) = setup_state().await;
    let store = state
        .store_for_workspace(session.workspace_id)
        .await
        .unwrap();
    let child = store
        .create_session(
            session.task_id,
            session.workspace_id,
            session.worktree_id,
            session.execution_environment,
            "fake".to_string(),
            "fake-model".to_string(),
            "subagent".to_string(),
            Some(session.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(child.id, session.workspace_id)
        .await
        .unwrap();
    assert!(store
        .archive_subagent_session(session.id, child.id)
        .await
        .unwrap());

    let read_store = store_for_existing_session_status_allow_archived(&state, child.id)
        .await
        .expect("history helper should allow archived child reads");
    assert!(
        read_store
            .get_session(child.id)
            .await
            .expect("load archived child session")
            .is_some(),
        "archived child session should remain readable for preserved history"
    );

    let api_store = store_for_existing_session_api_error_allow_archived(&state, child.id)
        .await
        .expect("history API helper should allow archived child reads");
    assert!(
        api_store
            .get_session(child.id)
            .await
            .expect("load archived child session via API helper")
            .is_some(),
        "archived child session should remain readable for preserved history"
    );
}

#[tokio::test]
async fn existing_session_write_store_helpers_reject_archived_subagents() {
    let (_data_dir, state, session) = setup_state().await;
    let store = state
        .store_for_workspace(session.workspace_id)
        .await
        .unwrap();
    let child = store
        .create_session(
            session.task_id,
            session.workspace_id,
            session.worktree_id,
            session.execution_environment,
            "fake".to_string(),
            "fake-model".to_string(),
            "subagent".to_string(),
            Some(session.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(child.id, session.workspace_id)
        .await
        .unwrap();
    assert!(store
        .archive_subagent_session(session.id, child.id)
        .await
        .unwrap());

    let result = store_for_existing_session_status_for_write(&state, child.id).await;
    match result {
        Ok(_) => panic!("expected archived subagent write helper to reject session"),
        Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
    }

    let result = store_for_existing_session_api_error_for_write(&state, child.id).await;
    match result {
        Ok(_) => panic!("expected archived subagent write API helper to reject session"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body.0.error, "session not found");
        }
    }
}
