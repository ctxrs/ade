use super::*;

#[tokio::test]
async fn worktree_bootstrap_logs_return_in_root_log_file() {
    let fixture = build_log_path_fixture().await;
    let (_task, session) = create_task_with_primary_session(&fixture).await;
    let store = fixture.state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("workspace worktree");

    let log_dir =
        ctx_observability::logs::logs_dir(fixture.data_dir.path()).join("worktree-bootstrap");
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("bootstrap.log");
    std::fs::write(&log_path, b"inside bootstrap log\n").unwrap();

    store
        .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: WorktreeBootstrapStatus::Success,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            exit_code: Some(0),
            timeout_sec: Some(60),
            error: None,
            log_path: Some(log_path.to_string_lossy().to_string()),
            log_truncated: Some(false),
            command: Some("true".to_string()),
            script_path: None,
        })
        .await
        .unwrap();

    let res = bootstrap_logs_response(&fixture.app, worktree.id).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"inside bootstrap log\n");
}

#[tokio::test]
async fn worktree_bootstrap_logs_fail_closed_for_legacy_outside_paths() {
    let fixture = build_log_path_fixture().await;
    let (_task, session) = create_task_with_primary_session(&fixture).await;
    let store = fixture.state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("workspace worktree");

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("bootstrap.log");
    std::fs::write(&outside_path, b"outside bootstrap log\n").unwrap();

    store
        .update_worktree_bootstrap_result(WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: WorktreeBootstrapStatus::Failed,
            started_at: Utc::now(),
            finished_at: Utc::now(),
            exit_code: Some(1),
            timeout_sec: Some(60),
            error: Some("legacy outside path".to_string()),
            log_path: Some(outside_path.to_string_lossy().to_string()),
            log_truncated: Some(false),
            command: Some("false".to_string()),
            script_path: None,
        })
        .await
        .unwrap();

    let res = bootstrap_logs_response(&fixture.app, worktree.id).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
