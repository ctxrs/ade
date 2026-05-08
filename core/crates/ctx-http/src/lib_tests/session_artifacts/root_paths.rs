use super::*;

#[tokio::test]
async fn session_artifacts_reject_outside_root_paths_and_fail_closed_for_legacy_rows() {
    let fixture = build_session_artifact_fixture().await;
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("outside.txt");
    std::fs::write(&outside_path, b"outside-body\n").unwrap();

    let res = post_session_artifacts(
        &fixture.app,
        fixture.session.id,
        json!([{
            "absolute_file_path": outside_path.to_string_lossy(),
            "name": "outside.txt",
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
            name: Some("outside.txt".to_string()),
            absolute_path: outside_path.to_string_lossy().to_string(),
            mime_type: "text/plain".to_string(),
            bytes: 13,
            created_at: chrono::Utc::now(),
            missing: None,
        })
        .await
        .unwrap();

    let session_state = get_session_state(&fixture.app, fixture.session.id).await;
    assert_eq!(
        session_state["artifacts"][0]["missing"].as_bool(),
        Some(true)
    );

    let res = get_session_artifact(&fixture.app, fixture.session.id, legacy.id).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
