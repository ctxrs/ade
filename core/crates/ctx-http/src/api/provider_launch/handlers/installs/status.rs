use super::*;

pub(in crate::api) async fn get_install(
    State(providers): State<ProvidersHandle>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    providers
        .get_provider_install_info(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct GetInstallStatusesReq {
    pub(in crate::api) install_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct InstallStatusBatchItem {
    pub(in crate::api) install_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api) info: Option<InstallInfo>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct GetInstallStatusesResp {
    pub(in crate::api) installs: Vec<InstallStatusBatchItem>,
}

pub(in crate::api) async fn get_install_statuses(
    State(providers): State<ProvidersHandle>,
    Json(req): Json<GetInstallStatusesReq>,
) -> Result<Json<GetInstallStatusesResp>, (StatusCode, Json<ApiErrorResp>)> {
    let install_ids = req
        .install_ids
        .into_iter()
        .map(|raw| {
            let parsed = uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("invalid install id: {raw}"),
                    }),
                )
            })?;
            Ok(InstallId::from(parsed))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut installs = Vec::with_capacity(install_ids.len());
    for install_id in install_ids {
        let info = providers.get_provider_install_info(install_id).await;
        installs.push(InstallStatusBatchItem {
            install_id: install_id.to_string(),
            info,
        });
    }

    Ok(Json(GetInstallStatusesResp { installs }))
}

pub(in crate::api) async fn cancel_install(
    State(providers): State<ProvidersHandle>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    providers
        .cancel_provider_install(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::api) async fn list_install_events(
    State(providers): State<ProvidersHandle>,
    Path(install_id): Path<String>,
) -> Result<Json<Vec<InstallProgressEvent>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    providers
        .list_provider_install_events(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
