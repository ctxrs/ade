use super::{
    default_mount_relpath, normalize_attachment_config, revision_key, sanitize_attachment_subpath,
    sanitize_mount_relpath, AttachmentConfig,
};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, WorkspaceAttachmentKind, WorkspaceAttachmentStatus,
};

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
