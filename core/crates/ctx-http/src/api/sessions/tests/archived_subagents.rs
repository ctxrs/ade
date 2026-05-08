use super::*;

async fn archived_subagent_fixture() -> (tempfile::TempDir, Arc<AppState>, Session, Session) {
    let (data_dir, state, session) = setup_state().await;
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
    (data_dir, state, session, child)
}

#[tokio::test]
async fn existing_session_read_store_helpers_reject_archived_subagents_by_default() {
    let (_data_dir, state, _session, child) = archived_subagent_fixture().await;

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
    let (_data_dir, state, _session, child) = archived_subagent_fixture().await;

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
    let (_data_dir, state, _session, child) = archived_subagent_fixture().await;

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
