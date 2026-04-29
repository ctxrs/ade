use crate::attachments::validate_mount_path_in_worktree;

use super::{
    resolve_attachment_source_path, sandbox_mount_parent_chain_validation_script,
    AttachmentSourceSymlinkPolicy,
};

#[tokio::test]
async fn resolve_attachment_source_path_rejects_parent_traversal() {
    let root = tempfile::tempdir().unwrap();
    let err = resolve_attachment_source_path(
        root.path(),
        Some("../escape.txt"),
        AttachmentSourceSymlinkPolicy::AllowInternal,
    )
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

        let err = resolve_attachment_source_path(
            root.path(),
            Some("link/escape.txt"),
            AttachmentSourceSymlinkPolicy::AllowInternal,
        )
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

        let err = resolve_attachment_source_path(
            root.path(),
            None,
            AttachmentSourceSymlinkPolicy::AllowInternal,
        )
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

        let resolved = resolve_attachment_source_path(
            root.path(),
            None,
            AttachmentSourceSymlinkPolicy::AllowInternal,
        )
        .await
        .unwrap();
        assert_eq!(resolved, root.path().canonicalize().unwrap());
    }
}

#[tokio::test]
async fn resolve_attachment_source_path_rejects_internal_symlinks_for_read_only_copy() {
    #[cfg(unix)]
    {
        let root = tempfile::tempdir().unwrap();
        let docs = root.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("guide.md"), b"guide").unwrap();
        std::os::unix::fs::symlink("guide.md", docs.join("guide-link")).unwrap();

        let err = resolve_attachment_source_path(
            root.path(),
            None,
            AttachmentSourceSymlinkPolicy::Reject,
        )
        .await
        .unwrap_err();

        assert!(format!("{err:#}").contains("refuses symlink"));
    }
}

#[tokio::test]
async fn sandbox_mount_validation_rejects_symlinked_ctx_parent() {
    #[cfg(unix)]
    {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join(".ctx")).unwrap();

        let target = worktree.join(".ctx/attachments/docs/docs");
        let err = validate_mount_path_in_worktree(&worktree, &target)
            .expect_err("sandbox mount validation should reject symlinked .ctx");

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }
}

#[tokio::test]
async fn sandbox_mount_validation_rejects_symlinked_attachments_parent() {
    #[cfg(unix)]
    {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(worktree.join(".ctx")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join(".ctx").join("attachments")).unwrap();

        let target = worktree.join(".ctx/attachments/docs/docs");
        let err = validate_mount_path_in_worktree(&worktree, &target)
            .expect_err("sandbox mount validation should reject symlinked attachments parent");

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }
}

#[tokio::test]
async fn sandbox_mount_validation_rejects_symlinked_docs_parent() {
    #[cfg(unix)]
    {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(worktree.join(".ctx/attachments")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join(".ctx/attachments/docs")).unwrap();

        let target = worktree.join(".ctx/attachments/docs/docs");
        let err = validate_mount_path_in_worktree(&worktree, &target)
            .expect_err("sandbox mount validation should reject symlinked docs parent");

        assert!(format!("{err:#}").contains("must not be a symlink"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_guest_parent_chain_script_rejects_symlinked_ctx_parent() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("worktree");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, worktree.join(".ctx")).unwrap();

    let target = worktree.join(".ctx/attachments/docs/docs");
    let output = std::process::Command::new("sh")
        .arg("-lc")
        .arg(sandbox_mount_parent_chain_validation_script())
        .arg("--")
        .arg(&worktree)
        .arg(&target)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("must not be a symlink"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !outside.join("attachments").exists(),
        "validation must not create through a symlinked guest parent"
    );

    let relative_output = std::process::Command::new("sh")
        .current_dir(&worktree)
        .arg("-lc")
        .arg(sandbox_mount_parent_chain_validation_script())
        .arg("--")
        .arg(".")
        .arg("./.ctx/attachments/docs/docs")
        .output()
        .unwrap();

    assert!(!relative_output.status.success());
    assert!(
        String::from_utf8_lossy(&relative_output.stderr).contains("must not be a symlink"),
        "stderr: {}",
        String::from_utf8_lossy(&relative_output.stderr)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_guest_parent_chain_script_allows_missing_safe_parents() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();

    let target = worktree.join(".ctx/attachments/docs/docs");
    let output = std::process::Command::new("sh")
        .arg("-lc")
        .arg(sandbox_mount_parent_chain_validation_script())
        .arg("--")
        .arg(&worktree)
        .arg(&target)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !worktree.join(".ctx").exists(),
        "validation should not create missing safe parents"
    );
}
