use super::*;

#[tokio::test]
async fn daemon_golden_path_with_fake_provider() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let (daemon, app, session) =
        build_fake_app_with_session(data_dir.path(), &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"content":"hello"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let produced = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session_store = daemon.store_for_session(session.id).await.unwrap();
            let msgs = session_store
                .list_messages_for_session(session.id)
                .await
                .unwrap();
            if msgs
                .iter()
                .any(|m| matches!(m.role, ctx_core::models::MessageRole::Assistant))
            {
                break;
            }
            let turns = session_store
                .list_session_turns_page_by_seq(session.id, None, Some(10))
                .await
                .unwrap();
            if turns.iter().any(|turn| {
                matches!(
                    turn.status,
                    ctx_core::models::SessionTurnStatus::Failed
                        | ctx_core::models::SessionTurnStatus::Interrupted
                )
            }) {
                panic!("turn failed before assistant message was produced: {turns:#?}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if produced.is_err() {
        let session_store = daemon.store_for_session(session.id).await.unwrap();
        let msgs = session_store
            .list_messages_for_session(session.id)
            .await
            .unwrap();
        let events = session_store.list_session_events(session.id).await.unwrap();
        let turns = session_store
            .list_session_turns_page_by_seq(session.id, None, Some(10))
            .await
            .unwrap();
        panic!(
            "assistant message not produced; messages={msgs:#?}; events={events:#?}; turns={turns:#?}"
        );
    }
}
