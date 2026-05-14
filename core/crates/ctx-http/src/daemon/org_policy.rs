use ctx_core::ids::{OrgId, WorkspaceId};
use ctx_core::models::{DaemonEnrollment, OrgPolicySnapshot, WorkspacePolicyOverlay};

use crate::daemon::{CoreHandle, WorkspaceStoreAccessError, WorkspacesHandle};

#[derive(Debug)]
pub(crate) enum WorkspacePolicyOverlayError {
    WorkspaceNotFound,
    Store(anyhow::Error),
}

fn workspace_policy_store_error(error: WorkspaceStoreAccessError) -> WorkspacePolicyOverlayError {
    match error {
        WorkspaceStoreAccessError::NotFound => WorkspacePolicyOverlayError::WorkspaceNotFound,
        WorkspaceStoreAccessError::Unavailable(error) => WorkspacePolicyOverlayError::Store(error),
    }
}

impl CoreHandle {
    pub(crate) async fn list_daemon_enrollments(&self) -> anyhow::Result<Vec<DaemonEnrollment>> {
        self.global_store().list_daemon_enrollments().await
    }

    pub(crate) async fn upsert_daemon_enrollment(
        &self,
        enrollment: DaemonEnrollment,
    ) -> anyhow::Result<DaemonEnrollment> {
        self.global_store()
            .upsert_daemon_enrollment(enrollment)
            .await
    }

    pub(crate) async fn get_daemon_enrollment_by_org_id(
        &self,
        org_id: OrgId,
    ) -> anyhow::Result<Option<DaemonEnrollment>> {
        self.global_store()
            .get_daemon_enrollment_by_org_id(org_id)
            .await
    }

    pub(crate) async fn upsert_org_policy_snapshot(
        &self,
        snapshot: OrgPolicySnapshot,
    ) -> anyhow::Result<OrgPolicySnapshot> {
        self.global_store()
            .upsert_org_policy_snapshot(snapshot)
            .await
    }
}

impl WorkspacesHandle {
    pub(crate) async fn get_workspace_policy_overlay(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspacePolicyOverlay>, WorkspacePolicyOverlayError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_policy_store_error)?;
        store
            .get_workspace_policy_overlay(workspace_id)
            .await
            .map_err(WorkspacePolicyOverlayError::Store)
    }

    pub(crate) async fn upsert_workspace_policy_overlay(
        &self,
        overlay: WorkspacePolicyOverlay,
    ) -> Result<WorkspacePolicyOverlay, WorkspacePolicyOverlayError> {
        let store = self
            .existing_workspace_store(overlay.workspace_id)
            .await
            .map_err(workspace_policy_store_error)?;
        store
            .upsert_workspace_policy_overlay(overlay)
            .await
            .map_err(WorkspacePolicyOverlayError::Store)
    }
}
