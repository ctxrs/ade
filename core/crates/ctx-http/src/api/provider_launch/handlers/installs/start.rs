use super::*;

pub(in crate::api) async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    let target =
        crate::daemon::installer::parse_install_target(query.target.as_deref()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": e.to_string()
                })),
            )
        })?;
    validate_install_target(target)?;

    let (install_id, _) = provider_launch_install::start_provider_install(&state, &id, target)
        .await
        .map_err(provider_install_error_response)?;

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
        target,
    }))
}

pub(in crate::api) async fn install_all_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, (StatusCode, Json<serde_json::Value>)> {
    let target =
        crate::daemon::installer::parse_install_target(query.target.as_deref()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid install target",
                })),
            )
        })?;
    validate_install_target(target)?;
    let installs = provider_launch_install::start_all_provider_installs(&state, target)
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

fn validate_install_target(
    target: ctx_provider_install::install_state::InstallTarget,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    ctx_settings_service::HostExecutionPolicy::current()
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": error.to_string()
                })),
            )
        })?
        .validate_install_target(target)
        .map_err(|error| {
            (
                crate::api::shared::status_code_for_request_or_policy_error(&error),
                Json(serde_json::json!({
                    "error": error.to_string()
                })),
            )
        })
}
