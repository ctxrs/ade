use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;

use super::*;

pub(super) struct ProviderVerifyOutcome {
    checked_at: String,
    status: String,
    auth_required: Option<bool>,
    message: Option<String>,
    endpoint_status: HarnessEndpointVerificationStatus,
    selected_endpoint_id: Option<String>,
    endpoint_catalog_result: bool,
}

impl ProviderVerifyOutcome {
    pub(super) fn new(checked_at: String, selected_endpoint_id: Option<String>) -> Self {
        Self {
            checked_at,
            status: "ok".to_string(),
            auth_required: Some(false),
            message: None,
            endpoint_status: HarnessEndpointVerificationStatus::Valid,
            selected_endpoint_id,
            endpoint_catalog_result: false,
        }
    }

    pub(super) fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    pub(super) fn selected_endpoint_id(&self) -> Option<&str> {
        self.selected_endpoint_id.as_deref()
    }

    pub(super) fn endpoint_status(&self) -> HarnessEndpointVerificationStatus {
        self.endpoint_status
    }

    pub(super) fn message(&self) -> Option<&String> {
        self.message.as_ref()
    }

    pub(super) fn has_endpoint_catalog_result(&self) -> bool {
        self.endpoint_catalog_result
    }

    pub(super) fn set_selected_endpoint_id(&mut self, selected_endpoint_id: Option<String>) {
        self.selected_endpoint_id = selected_endpoint_id;
    }

    pub(super) fn apply_unusable_provider(&mut self, message: String) {
        self.status = "error".to_string();
        self.auth_required = Some(false);
        self.message = Some(message);
        self.endpoint_status = HarnessEndpointVerificationStatus::Error;
    }

    pub(super) fn apply_endpoint_catalog_refresh(
        &mut self,
        refreshed_endpoint: HarnessEndpointRecord,
    ) {
        self.endpoint_catalog_result = true;
        self.selected_endpoint_id = Some(refreshed_endpoint.id.clone());
        let (status, auth_required, message, endpoint_status) =
            endpoint_catalog_verify_outcome(&refreshed_endpoint);
        self.status = status;
        self.auth_required = auth_required;
        self.message = message;
        self.endpoint_status = endpoint_status;
    }

    pub(super) fn apply_classified_probe_error(&mut self, message: String) {
        let (status, auth_required, endpoint_status) = classify_probe_error(&message);
        self.status = status.to_string();
        self.auth_required = auth_required;
        self.message = Some(message);
        self.endpoint_status = endpoint_status;
    }

    pub(super) fn apply_endpoint_catalog_runtime_probe_failure(&mut self, message: String) {
        let (status, auth_required, message, endpoint_status) =
            endpoint_catalog_runtime_probe_failure(message, self.endpoint_status);
        self.status = status;
        self.auth_required = auth_required;
        self.message = message;
        self.endpoint_status = endpoint_status;
    }

    pub(super) fn into_snapshot(
        self,
        provider_id: &str,
        ws_id: WorkspaceId,
    ) -> ProviderAuthCheckSnapshot {
        ProviderAuthCheckSnapshot {
            provider_id: provider_id.to_string(),
            workspace_id: ws_id.0.to_string(),
            status: self.status,
            auth_required: self.auth_required,
            checked_at: Some(self.checked_at),
            message: self.message,
        }
    }
}

pub(super) fn config_error_snapshot(
    provider_id: &str,
    ws_id: WorkspaceId,
    checked_at: &str,
    message: String,
) -> ProviderAuthCheckSnapshot {
    ProviderAuthCheckSnapshot {
        provider_id: provider_id.to_string(),
        workspace_id: ws_id.0.to_string(),
        status: "error".to_string(),
        auth_required: Some(false),
        checked_at: Some(checked_at.to_string()),
        message: Some(message),
    }
}
