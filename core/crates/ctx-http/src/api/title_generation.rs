use super::*;
use crate::daemon::sessions::title_generation as daemon_title_generation;

#[derive(Debug, Serialize)]
pub(in crate::api) struct TitleGenerationLocalStatusResponse {
    pub ready: bool,
    pub runtime: daemon_title_generation::TitleGenerationLocalRuntimeStatus,
    pub model: daemon_title_generation::TitleGenerationLocalModelStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_id: Option<InstallId>,
    pub install_running: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct TitleGenerationLocalInstallResponse {
    pub install_id: InstallId,
}

pub(in crate::api) async fn get_title_generation_local_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalStatusResponse>, StatusCode> {
    let status = daemon_title_generation::title_generation_local_status(&state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(TitleGenerationLocalStatusResponse {
        ready: status.ready,
        runtime: status.runtime,
        model: status.model,
        install_id: status.install_id,
        install_running: status.install_running,
    }))
}

pub(in crate::api) async fn install_title_generation_local(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalInstallResponse>, StatusCode> {
    let install_id = daemon_title_generation::start_title_generation_local_install(state).await;
    Ok(Json(TitleGenerationLocalInstallResponse { install_id }))
}
