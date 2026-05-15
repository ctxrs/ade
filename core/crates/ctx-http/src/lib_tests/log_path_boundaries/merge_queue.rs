use super::*;

#[tokio::test]
async fn merge_queue_entry_logs_return_in_root_log_file() {
    let fixture = build_log_path_fixture().await;
    let log_dir = fixture
        .git_repo
        .path()
        .join(".ctx")
        .join("merge-queue")
        .join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("merge-queue.log");
    std::fs::write(&log_path, b"inside merge queue log\n").unwrap();

    let entry_id = fixture
        .daemon
        .seed_failed_merge_queue_log_run_for_test(
            fixture.workspace.id,
            "inside log",
            &log_path,
            "inside path",
        )
        .await
        .unwrap();

    let res = merge_queue_logs_response(&fixture.app, fixture.workspace.id, entry_id).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"inside merge queue log\n");
}

#[tokio::test]
async fn merge_queue_entry_logs_fail_closed_for_legacy_outside_paths() {
    let fixture = build_log_path_fixture().await;

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("merge-queue.log");
    std::fs::write(&outside_path, b"outside merge queue log\n").unwrap();

    let entry_id = fixture
        .daemon
        .seed_failed_merge_queue_log_run_for_test(
            fixture.workspace.id,
            "legacy outside log",
            &outside_path,
            "legacy outside path",
        )
        .await
        .unwrap();

    let res = merge_queue_logs_response(&fixture.app, fixture.workspace.id, entry_id).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
