use super::*;

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
