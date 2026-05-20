use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::{HarnessEndpointRecord, HarnessEndpointVerificationStatus};
use serde::Serialize;

use crate::provider_launch::models::{
    endpoint_catalog_runtime_probe_failure, endpoint_catalog_verify_outcome,
};
use crate::provider_launch::probe_error::classify_probe_error;

#[derive(Clone, Debug, Serialize)]
pub struct ProviderAuthCheckSnapshot {
    pub provider_id: String,
    pub workspace_id: String,
    pub status: String,
    pub auth_required: Option<bool>,
    pub checked_at: Option<String>,
    pub message: Option<String>,
}

pub struct ProviderVerifyOutcome {
    checked_at: String,
    status: String,
    auth_required: Option<bool>,
    message: Option<String>,
    endpoint_status: HarnessEndpointVerificationStatus,
    selected_endpoint_id: Option<String>,
    endpoint_catalog_result: bool,
}

impl ProviderVerifyOutcome {
    pub fn new(checked_at: String, selected_endpoint_id: Option<String>) -> Self {
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

    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn selected_endpoint_id(&self) -> Option<&str> {
        self.selected_endpoint_id.as_deref()
    }

    pub fn endpoint_status(&self) -> HarnessEndpointVerificationStatus {
        self.endpoint_status
    }

    pub fn message(&self) -> Option<&String> {
        self.message.as_ref()
    }

    pub fn has_endpoint_catalog_result(&self) -> bool {
        self.endpoint_catalog_result
    }

    pub fn set_selected_endpoint_id(&mut self, selected_endpoint_id: Option<String>) {
        self.selected_endpoint_id = selected_endpoint_id;
    }

    pub fn apply_unusable_provider(&mut self, message: String) {
        self.status = "error".to_string();
        self.auth_required = Some(false);
        self.message = Some(message);
        self.endpoint_status = HarnessEndpointVerificationStatus::Error;
    }

    pub fn apply_endpoint_catalog_refresh(&mut self, refreshed_endpoint: HarnessEndpointRecord) {
        self.endpoint_catalog_result = true;
        self.selected_endpoint_id = Some(refreshed_endpoint.id.clone());
        let (status, auth_required, message, endpoint_status) =
            endpoint_catalog_verify_outcome(&refreshed_endpoint);
        self.status = status;
        self.auth_required = auth_required;
        self.message = message;
        self.endpoint_status = endpoint_status;
    }

    pub fn apply_classified_probe_error(&mut self, message: String) {
        let (status, auth_required, endpoint_status) = classify_probe_error(&message);
        self.status = status.to_string();
        self.auth_required = auth_required;
        self.message = Some(message);
        self.endpoint_status = endpoint_status;
    }

    pub fn apply_endpoint_catalog_runtime_probe_failure(&mut self, message: String) {
        let (status, auth_required, message, endpoint_status) =
            endpoint_catalog_runtime_probe_failure(message, self.endpoint_status);
        self.status = status;
        self.auth_required = auth_required;
        self.message = message;
        self.endpoint_status = endpoint_status;
    }

    pub fn into_snapshot(self, provider_id: &str, ws_id: WorkspaceId) -> ProviderAuthCheckSnapshot {
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

pub fn config_error_snapshot(
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
