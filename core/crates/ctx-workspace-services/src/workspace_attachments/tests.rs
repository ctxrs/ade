use super::{
    AttachmentConfig, default_mount_relpath, materialize_attachment, normalize_attachment_config,
    resolve_workspace_local_source, revision_key, sanitize_attachment_subpath,
    sanitize_mount_relpath,
};
use chrono::Utc;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus,
};
use std::path::Path;

fn test_workspace(root: &Path) -> Workspace {
    Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    }
}

#[test]
fn default_mount_relpath_uses_kind_specific_roots() {
    assert_eq!(
        default_mount_relpath(&WorkspaceAttachmentKind::ReferenceRepo, "My Docs"),
        ".ctx/attachments/refs/my-docs"
    );
    assert_eq!(
        default_mount_relpath(&WorkspaceAttachmentKind::DocMirror, "API Guide"),
        ".ctx/attachments/docs/api-guide"
    );
}

#[test]
fn sanitize_mount_relpath_rejects_invalid_paths() {
    assert!(sanitize_mount_relpath(".ctx/attachments/docs/api-guide").is_ok());
    assert!(sanitize_mount_relpath("").is_err());
    assert!(sanitize_mount_relpath("/absolute/path").is_err());
    assert!(sanitize_mount_relpath("../escape").is_err());
}

#[test]
fn sanitize_attachment_subpath_rejects_escape_paths() {
    assert!(sanitize_attachment_subpath("guide/index.md").is_ok());
    assert!(sanitize_attachment_subpath("").is_err());
    assert!(sanitize_attachment_subpath("/absolute/path").is_err());
    assert!(sanitize_attachment_subpath("../escape").is_err());
    assert!(sanitize_attachment_subpath("guide/../../escape").is_err());
}

#[test]
fn normalize_attachment_config_preserves_existing_identity() {
    let workspace_id = WorkspaceId::new();
    let existing = normalize_attachment_config(
        workspace_id,
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "https://example.com/repo.git".to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: Some(AttachmentMode::Ro),
            update_policy: Some(AttachmentUpdatePolicy::Manual),
        },
        None,
    )
    .unwrap();

    let updated = normalize_attachment_config(
        workspace_id,
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "https://example.com/repo.git".to_string(),
            revision: Some("main".to_string()),
            subpath: Some("guide".to_string()),
            mount_relpath: Some(".ctx/attachments/refs/docs".to_string()),
            mode: Some(AttachmentMode::Ro),
            update_policy: Some(AttachmentUpdatePolicy::OnOpen),
        },
        Some(existing.clone()),
    )
    .unwrap();

    assert_eq!(updated.id, existing.id);
    assert_eq!(updated.created_at, existing.created_at);
    assert_eq!(updated.status, WorkspaceAttachmentStatus::Pending);
    assert_eq!(updated.mount_relpath, ".ctx/attachments/refs/docs");
    assert_eq!(updated.revision.as_deref(), Some("main"));
    assert_eq!(updated.subpath.as_deref(), Some("guide"));
    assert_eq!(updated.update_policy, AttachmentUpdatePolicy::OnOpen);
}

#[test]
fn revision_key_defaults_and_sanitizes() {
    let attachment = normalize_attachment_config(
        WorkspaceId::new(),
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::DocMirror,
            name: "Docs".to_string(),
            source: "https://example.com".to_string(),
            revision: Some("Feature/Branch".to_string()),
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(revision_key(&attachment), "feature-branch");
}

#[test]
fn normalize_attachment_config_rejects_blank_source() {
    let err = normalize_attachment_config(
        WorkspaceId::new(),
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "   ".to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .expect_err("blank attachment source should fail");

    assert!(format!("{err:#}").contains("source must not be empty"));
}

#[test]
fn normalize_reference_repo_local_source_requires_absolute_path() {
    let err = normalize_attachment_config(
        WorkspaceId::new(),
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "../references/docs".to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .expect_err("relative local reference repo source should fail");

    assert!(format!("{err:#}").contains("absolute path or repository URL"));
}

#[test]
fn normalize_reference_repo_accepts_scp_style_ssh_urls() {
    let attachment = normalize_attachment_config(
        WorkspaceId::new(),
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "git@github.com:openai/ctx.git".to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .expect("scp-style ssh source should remain supported");

    assert_eq!(attachment.source, "git@github.com:openai/ctx.git");
}

#[test]
fn normalize_reference_repo_accepts_host_alias_scp_urls() {
    let attachment = normalize_attachment_config(
        WorkspaceId::new(),
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "Docs".to_string(),
            source: "corp-git:team/repo.git".to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .expect("host-alias scp source should remain supported");

    assert_eq!(attachment.source, "corp-git:team/repo.git");
}

#[test]
fn resolve_workspace_local_source_allows_workspace_absolute_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let nested = workspace
        .path()
        .join(".ctx")
        .join("scripts")
        .join("docs.py");
    std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
    std::fs::write(&nested, "print('ok')\n").unwrap();

    let resolved = resolve_workspace_local_source(
        workspace.path(),
        &nested.to_string_lossy(),
        "doc mirror script",
    )
    .expect("workspace-local absolute path should resolve");

    assert_eq!(resolved, std::fs::canonicalize(nested).unwrap());
}

#[test]
fn resolve_workspace_local_source_rejects_paths_outside_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_script = outside.path().join("docs.py");
    std::fs::write(&outside_script, "print('bad')\n").unwrap();

    let err = resolve_workspace_local_source(
        workspace.path(),
        &outside_script.to_string_lossy(),
        "doc mirror script",
    )
    .expect_err("outside path should fail");

    assert!(format!("{err:#}").contains("must stay within workspace root"));
}

#[tokio::test]
async fn materialize_doc_mirror_rejects_script_outside_workspace_without_executing() {
    let workspace_dir = tempfile::tempdir().unwrap();
    let workspace = test_workspace(workspace_dir.path());
    let outside = tempfile::tempdir().unwrap();
    let marker = outside.path().join("ran.txt");
    let script_path = outside.path().join("docs.py");
    std::fs::write(
        &script_path,
        format!(
            "from pathlib import Path\nPath({:?}).write_text('ran', encoding='utf-8')\n",
            marker.to_string_lossy()
        ),
    )
    .unwrap();
    let data_root = tempfile::tempdir().unwrap();
    let attachment = normalize_attachment_config(
        workspace.id,
        AttachmentConfig {
            kind: WorkspaceAttachmentKind::DocMirror,
            name: "Docs".to_string(),
            source: script_path.to_string_lossy().to_string(),
            revision: None,
            subpath: None,
            mount_relpath: None,
            mode: None,
            update_policy: None,
        },
        None,
    )
    .unwrap();

    let err = materialize_attachment(data_root.path(), &workspace, &attachment, true)
        .await
        .expect_err("outside doc mirror script should fail");

    assert!(format!("{err:#}").contains("must stay within workspace root"));
    assert!(!marker.exists(), "outside script should not have executed");
}
