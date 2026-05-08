use super::*;

#[tokio::test]
async fn session_artifacts_do_not_accept_other_session_spool_files() {
    let fixture = build_session_artifact_fixture().await;
    let other_session =
        create_subagent_session_via_api(&fixture.app, &fixture.task, fixture.session.id).await;

    let other_spool_dir = fixture
        .state
        .core
        .tool_output_spool_dir
        .join(other_session.id.0.to_string())
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&other_spool_dir).unwrap();
    let other_spool_path = other_spool_dir.join("foreign.txt");
    std::fs::write(&other_spool_path, b"other-session-spool\n").unwrap();

    let store = fixture
        .state
        .store_for_session(fixture.session.id)
        .await
        .unwrap();
    let legacy = store
        .upsert_session_artifact_by_path(&ctx_core::models::Artifact {
            id: ctx_core::ids::ArtifactId::new(),
            session_id: fixture.session.id,
            task_id: fixture.session.task_id,
            workspace_id: fixture.session.workspace_id,
            worktree_id: fixture.session.worktree_id,
            name: Some("foreign.txt".to_string()),
            absolute_path: other_spool_path.to_string_lossy().to_string(),
            mime_type: "text/plain".to_string(),
            bytes: 20,
            created_at: chrono::Utc::now(),
            missing: None,
        })
        .await
        .unwrap();

    let res = post_session_artifacts(
        &fixture.app,
        fixture.session.id,
        json!([{
            "absolute_file_path": other_spool_path.to_string_lossy(),
            "name": "foreign.txt",
            "mime_type": "text/plain"
        }]),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"].as_str(),
        Some("artifact 1 absolute_file_path must stay inside the session worktree or tool-output spool")
    );

    let session_state = get_session_state(&fixture.app, fixture.session.id).await;
    assert_eq!(
        session_state["artifacts"][0]["missing"].as_bool(),
        Some(true)
    );

    let res = get_session_artifact(&fixture.app, fixture.session.id, legacy.id).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
