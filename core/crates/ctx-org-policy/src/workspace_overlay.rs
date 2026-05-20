use ctx_core::ids::OrgId;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::WorkspacePolicyOverlay;
use ctx_store::Store;

#[derive(Debug)]
pub enum WorkspacePolicyOverlayError {
    WorkspaceNotFound,
    Store(anyhow::Error),
}

#[derive(Debug)]
pub enum UpsertWorkspacePolicyOverlayError {
    EnrollmentMissing,
    EnrollmentLoad(anyhow::Error),
    WorkspaceNotFound,
    Store(anyhow::Error),
}

pub fn upsert_workspace_policy_overlay_error(
    error: WorkspacePolicyOverlayError,
) -> UpsertWorkspacePolicyOverlayError {
    match error {
        WorkspacePolicyOverlayError::WorkspaceNotFound => {
            UpsertWorkspacePolicyOverlayError::WorkspaceNotFound
        }
        WorkspacePolicyOverlayError::Store(error) => {
            UpsertWorkspacePolicyOverlayError::Store(error)
        }
    }
}

pub async fn get_workspace_policy_overlay(
    store: &Store,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspacePolicyOverlay>, WorkspacePolicyOverlayError> {
    store
        .get_workspace_policy_overlay(workspace_id)
        .await
        .map_err(WorkspacePolicyOverlayError::Store)
}

pub async fn upsert_workspace_policy_overlay(
    store: &Store,
    overlay: WorkspacePolicyOverlay,
) -> Result<WorkspacePolicyOverlay, WorkspacePolicyOverlayError> {
    store
        .upsert_workspace_policy_overlay(overlay)
        .await
        .map_err(WorkspacePolicyOverlayError::Store)
}

pub async fn validate_daemon_enrollment_for_overlay(
    global_store: &Store,
    org_id: OrgId,
) -> Result<(), UpsertWorkspacePolicyOverlayError> {
    let enrollment = global_store
        .get_daemon_enrollment_by_org_id(org_id)
        .await
        .map_err(UpsertWorkspacePolicyOverlayError::EnrollmentLoad)?;
    if enrollment.is_none() {
        return Err(UpsertWorkspacePolicyOverlayError::EnrollmentMissing);
    }
    Ok(())
}

pub async fn upsert_workspace_policy_overlay_checked(
    global_store: &Store,
    workspace_store: &Store,
    overlay: WorkspacePolicyOverlay,
) -> Result<WorkspacePolicyOverlay, UpsertWorkspacePolicyOverlayError> {
    validate_daemon_enrollment_for_overlay(global_store, overlay.org_id).await?;
    upsert_workspace_policy_overlay(workspace_store, overlay)
        .await
        .map_err(upsert_workspace_policy_overlay_error)
}
