use super::*;

#[tokio::test]
async fn merge_queue_entry_logs_return_in_root_log_file() {
    let fixture = build_log_path_fixture().await;
    let store = fixture
        .state
        .store_for_workspace(fixture.workspace.id)
        .await
        .unwrap();

    let entry = merge_queue_entry(fixture.workspace.id, "inside log");
    store.create_merge_queue_entry(&entry).await.unwrap();

    let log_dir = fixture
        .git_repo
        .path()
        .join(".ctx")
        .join("merge-queue")
        .join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("merge-queue.log");
    std::fs::write(&log_path, b"inside merge queue log\n").unwrap();

    let run = merge_queue_run(
        entry.id,
        log_path.to_string_lossy().to_string(),
        "inside path",
    );
    store.create_merge_queue_run(&run).await.unwrap();

    let res = merge_queue_logs_response(&fixture.app, fixture.workspace.id, entry.id).await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"inside merge queue log\n");
}

#[tokio::test]
async fn merge_queue_entry_logs_fail_closed_for_legacy_outside_paths() {
    let fixture = build_log_path_fixture().await;
    let store = fixture
        .state
        .store_for_workspace(fixture.workspace.id)
        .await
        .unwrap();

    let entry = merge_queue_entry(fixture.workspace.id, "legacy outside log");
    store.create_merge_queue_entry(&entry).await.unwrap();

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("merge-queue.log");
    std::fs::write(&outside_path, b"outside merge queue log\n").unwrap();

    let run = merge_queue_run(
        entry.id,
        outside_path.to_string_lossy().to_string(),
        "legacy outside path",
    );
    store.create_merge_queue_run(&run).await.unwrap();

    let res = merge_queue_logs_response(&fixture.app, fixture.workspace.id, entry.id).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
