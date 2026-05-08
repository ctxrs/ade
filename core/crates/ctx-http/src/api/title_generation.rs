use super::*;

pub(in crate::api) const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";

#[derive(Debug, Serialize)]
pub(in crate::api) struct TitleGenerationLocalStatusResponse {
    pub ready: bool,
    pub runtime: title_generation_local::TitleGenerationLocalRuntimeStatus,
    pub model: title_generation_local::TitleGenerationLocalModelStatus,
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
    let status = title_generation_local::local_status(&state.core.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let install_id = state
        .find_running_install(TITLE_GENERATION_LOCAL_INSTALL_KEY, None)
        .await;
    Ok(Json(TitleGenerationLocalStatusResponse {
        ready: status.ready,
        runtime: status.runtime,
        model: status.model,
        install_id,
        install_running: install_id.is_some(),
    }))
}

pub(in crate::api) async fn install_title_generation_local(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalInstallResponse>, StatusCode> {
    let (install_id, started_new) = state
        .start_install(TITLE_GENERATION_LOCAL_INSTALL_KEY.to_string(), None)
        .await;
    if started_new {
        let state2 = state.clone();
        tokio::spawn(async move {
            if let Err(e) =
                installer::install_title_generation_local_with_progress(state2.clone(), install_id)
                    .await
            {
                tracing::error!("local title generation install failed: {e:#}");
            }
        });
    }

    Ok(Json(TitleGenerationLocalInstallResponse { install_id }))
}
