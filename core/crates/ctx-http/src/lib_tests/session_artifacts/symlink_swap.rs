use super::*;

#[tokio::test]
async fn open_canonical_session_artifact_file_rejects_symlink_swap() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("outside.txt");
    std::fs::write(&outside_path, b"outside\n").unwrap();

    let artifact_path = root.path().join("artifact.txt");
    std::fs::write(&artifact_path, b"inside\n").unwrap();
    let canonical = tokio::fs::canonicalize(&artifact_path).await.unwrap();

    let renamed = root.path().join("artifact.saved");
    std::fs::rename(&artifact_path, &renamed).unwrap();
    std::os::unix::fs::symlink(&outside_path, &artifact_path).unwrap();

    let err = crate::api::open_canonical_session_artifact_file(&canonical)
        .await
        .unwrap_err();
    assert_eq!(err, StatusCode::NOT_FOUND);
}
