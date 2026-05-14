use super::*;

pub(in crate::api) async fn install_provider(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    let target = parse_install_target_query(query.target.as_deref())?;

    let install_id = providers
        .start_provider_install(&id, target)
        .await
        .map_err(provider_install_error_response)?;

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
        target,
    }))
}

pub(in crate::api) async fn install_all_providers(
    State(providers): State<ProvidersHandle>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, (StatusCode, Json<serde_json::Value>)> {
    let target = parse_install_target_query(query.target.as_deref())?;
    let installs = providers
        .start_all_provider_installs(target)
        .await
        .map_err(provider_install_error_response)?;
    Ok(Json(
        installs
            .into_iter()
            .map(|(provider_id, install_id)| InstallStartResponse {
                provider_id,
                install_id,
                target,
            })
            .collect(),
    ))
}

fn parse_install_target_query(
    raw: Option<&str>,
) -> Result<InstallTarget, (StatusCode, Json<serde_json::Value>)> {
    parse_provider_install_target(raw).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": error,
            })),
        )
    })
}
