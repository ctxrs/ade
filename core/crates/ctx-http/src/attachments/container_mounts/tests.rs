use super::resolve_attachment_source_path;

#[tokio::test]
async fn resolve_attachment_source_path_rejects_parent_traversal() {
    let root = tempfile::tempdir().unwrap();
    let err = resolve_attachment_source_path(root.path(), Some("../escape.txt"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("attachment source not found"));
}

#[tokio::test]
async fn resolve_attachment_source_path_rejects_symlink_escape() {
    #[cfg(unix)]
    {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("escape.txt"), b"escape").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();

        let err = resolve_attachment_source_path(root.path(), Some("link/escape.txt"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("escapes the materialized root"));
    }
}
