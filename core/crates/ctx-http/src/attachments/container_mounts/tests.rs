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

#[tokio::test]
async fn resolve_attachment_source_path_rejects_nested_symlink_escape() {
    #[cfg(unix)]
    {
        let root = tempfile::tempdir().unwrap();
        let docs = root.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("escape.txt"), b"escape").unwrap();
        std::os::unix::fs::symlink(outside.path().join("escape.txt"), docs.join("leak")).unwrap();

        let err = resolve_attachment_source_path(root.path(), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("escapes the materialized root"));
    }
}

#[tokio::test]
async fn resolve_attachment_source_path_allows_internal_symlinks() {
    #[cfg(unix)]
    {
        let root = tempfile::tempdir().unwrap();
        let docs = root.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("guide.md"), b"guide").unwrap();
        std::os::unix::fs::symlink("guide.md", docs.join("guide-link")).unwrap();

        let resolved = resolve_attachment_source_path(root.path(), None)
            .await
            .unwrap();
        assert_eq!(resolved, root.path().canonicalize().unwrap());
    }
}
