use std::sync::Arc;

use ctx_observability::logs;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::daemon::{DaemonState, ProvidersHandle};

pub use ctx_provider_runtime::provider_launch::install::StartProviderInstallError;

pub type ProviderInstallInfo = InstallInfo;
pub type ProviderInstallProgressEvent = InstallProgressEvent;

#[derive(Debug, Serialize)]
pub struct ProviderInstallStartRouteResponse {
    pub provider_id: String,
    pub install_id: InstallId,
    pub target: InstallTarget,
}

#[derive(Debug, Clone, Copy)]
pub enum ProviderInstallJsonRouteErrorStatus {
    BadRequest,
    Forbidden,
}

#[derive(Debug)]
pub struct ProviderInstallJsonRouteError {
    status: ProviderInstallJsonRouteErrorStatus,
    body: Value,
}

impl ProviderInstallJsonRouteError {
    pub fn new(status: ProviderInstallJsonRouteErrorStatus, body: Value) -> Self {
        Self { status, body }
    }

    pub fn status(&self) -> &ProviderInstallJsonRouteErrorStatus {
        &self.status
    }

    pub fn body(&self) -> &Value {
        &self.body
    }

    fn bad_request_error(message: String) -> Self {
        Self {
            status: ProviderInstallJsonRouteErrorStatus::BadRequest,
            body: serde_json::json!({
                "error": message,
            }),
        }
    }

    fn start_error(error: StartProviderInstallError) -> Self {
        let status = if error.code.as_deref() == Some("install_target_disabled") {
            ProviderInstallJsonRouteErrorStatus::Forbidden
        } else {
            ProviderInstallJsonRouteErrorStatus::BadRequest
        };
        Self {
            status,
            body: serde_json::json!({
                "error": logs::redact_sensitive(&error.message),
                "code": error.code,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProviderInstallStatusesRouteRequest {
    pub install_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInstallStatusBatchItem {
    pub install_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<InstallInfo>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInstallStatusesRouteResponse {
    pub installs: Vec<ProviderInstallStatusBatchItem>,
}

#[derive(Debug)]
pub enum ProviderInstallStatusOnlyRouteError {
    BadRequest,
    NotFound,
}

pub struct ProviderInstallEventStreamRoute {
    pub history: Vec<InstallProgressEvent>,
    pub receiver: broadcast::Receiver<InstallProgressEvent>,
}

pub fn parse_provider_install_target(raw: Option<&str>) -> Result<InstallTarget, String> {
    ctx_managed_installs::parse_install_target(raw).map_err(|error| error.to_string())
}

pub async fn start_provider_install(
    state: &Arc<DaemonState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<InstallId, StartProviderInstallError> {
    let (install_id, _) = ctx_provider_runtime::provider_launch::install::start_provider_install(
        state,
        provider_id,
        target,
    )
    .await?;
    Ok(install_id)
}

pub async fn start_all_provider_installs(
    state: &Arc<DaemonState>,
    target: InstallTarget,
) -> Result<Vec<(String, InstallId)>, StartProviderInstallError> {
    ctx_provider_runtime::provider_launch::install::start_all_provider_installs(state, target).await
}

pub async fn get_provider_install_info(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.get_install_polling_info(install_id).await
}

pub async fn cancel_provider_install(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.cancel_install(install_id).await
}

pub async fn list_provider_install_events(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<Vec<InstallProgressEvent>> {
    state.get_install_events(install_id).await
}

pub async fn provider_install_event_sender(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<broadcast::Sender<InstallProgressEvent>> {
    state.get_install_sender(install_id).await
}

impl ProvidersHandle {
    pub async fn start_provider_install_for_route(
        &self,
        provider_id: &str,
        raw_target: Option<&str>,
    ) -> Result<ProviderInstallStartRouteResponse, ProviderInstallJsonRouteError> {
        let target = parse_provider_install_target(raw_target)
            .map_err(ProviderInstallJsonRouteError::bad_request_error)?;
        let install_id = start_provider_install(&self.state, provider_id, target)
            .await
            .map_err(ProviderInstallJsonRouteError::start_error)?;
        Ok(ProviderInstallStartRouteResponse {
            provider_id: provider_id.to_string(),
            install_id,
            target,
        })
    }

    pub async fn start_all_provider_installs_for_route(
        &self,
        raw_target: Option<&str>,
    ) -> Result<Vec<ProviderInstallStartRouteResponse>, ProviderInstallJsonRouteError> {
        let target = parse_provider_install_target(raw_target)
            .map_err(ProviderInstallJsonRouteError::bad_request_error)?;
        let installs = start_all_provider_installs(&self.state, target)
            .await
            .map_err(ProviderInstallJsonRouteError::start_error)?;
        Ok(installs
            .into_iter()
            .map(
                |(provider_id, install_id)| ProviderInstallStartRouteResponse {
                    provider_id,
                    install_id,
                    target,
                },
            )
            .collect())
    }

    pub async fn get_provider_install_for_route(
        &self,
        raw_install_id: &str,
    ) -> Result<InstallInfo, ProviderInstallStatusOnlyRouteError> {
        let install_id = parse_install_id_for_status_route(raw_install_id)?;
        get_provider_install_info(&self.state, install_id)
            .await
            .ok_or(ProviderInstallStatusOnlyRouteError::NotFound)
    }

    pub async fn get_provider_install_statuses_for_route(
        &self,
        request: ProviderInstallStatusesRouteRequest,
    ) -> Result<ProviderInstallStatusesRouteResponse, ProviderInstallJsonRouteError> {
        let install_ids = request
            .install_ids
            .into_iter()
            .map(|raw| {
                let parsed = uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
                    ProviderInstallJsonRouteError::bad_request_error(format!(
                        "invalid install id: {raw}"
                    ))
                })?;
                Ok(InstallId::from(parsed))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut installs = Vec::with_capacity(install_ids.len());
        for install_id in install_ids {
            let info = get_provider_install_info(&self.state, install_id).await;
            installs.push(ProviderInstallStatusBatchItem {
                install_id: install_id.to_string(),
                info,
            });
        }

        Ok(ProviderInstallStatusesRouteResponse { installs })
    }

    pub async fn cancel_provider_install_for_route(
        &self,
        raw_install_id: &str,
    ) -> Result<InstallInfo, ProviderInstallStatusOnlyRouteError> {
        let install_id = parse_install_id_for_status_route(raw_install_id)?;
        cancel_provider_install(&self.state, install_id)
            .await
            .ok_or(ProviderInstallStatusOnlyRouteError::NotFound)
    }

    pub async fn list_provider_install_events_for_route(
        &self,
        raw_install_id: &str,
    ) -> Result<Vec<InstallProgressEvent>, ProviderInstallStatusOnlyRouteError> {
        let install_id = parse_install_id_for_status_route(raw_install_id)?;
        list_provider_install_events(&self.state, install_id)
            .await
            .ok_or(ProviderInstallStatusOnlyRouteError::NotFound)
    }

    pub async fn open_provider_install_event_stream_for_route(
        &self,
        raw_install_id: &str,
    ) -> Result<ProviderInstallEventStreamRoute, ProviderInstallStatusOnlyRouteError> {
        let install_id = parse_install_id_for_status_route(raw_install_id)?;
        let Some(sender) = provider_install_event_sender(&self.state, install_id).await else {
            return Err(ProviderInstallStatusOnlyRouteError::NotFound);
        };
        let history = list_provider_install_events(&self.state, install_id)
            .await
            .unwrap_or_default();
        Ok(ProviderInstallEventStreamRoute {
            history,
            receiver: sender.subscribe(),
        })
    }
}

fn parse_install_id_for_status_route(
    raw_install_id: &str,
) -> Result<InstallId, ProviderInstallStatusOnlyRouteError> {
    raw_install_id
        .parse::<InstallId>()
        .map_err(|_| ProviderInstallStatusOnlyRouteError::BadRequest)
}
