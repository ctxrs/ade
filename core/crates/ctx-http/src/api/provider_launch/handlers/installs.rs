use super::*;

pub(in crate::api) async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    let requested_provider_id = id;
    let id = canonicalize_provider_id(&requested_provider_id);
    let target = crate::installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
    })?;
    crate::execution_policy::HostExecutionPolicy::current()
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
        })?;

    let (install_id, _) =
        provider_launch_install::start_provider_install(&state, &id, target).await?;

    Ok(Json(InstallStartResponse {
        provider_id: project_provider_id_for_response(&requested_provider_id, &id),
        install_id,
        target,
    }))
}

pub(in crate::api) async fn install_all_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, (StatusCode, Json<serde_json::Value>)> {
    let target = crate::installer::parse_install_target(query.target.as_deref()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid install target",
            })),
        )
    })?;
    crate::execution_policy::HostExecutionPolicy::current()
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
        })?;
    let installs = provider_launch_install::start_all_provider_installs(&state, target).await?;
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

pub(in crate::api) async fn get_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_polling_info(install_id)
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
    State(state): State<Arc<AppState>>,
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
        let info = state.get_install_polling_info(install_id).await;
        installs.push(InstallStatusBatchItem {
            install_id: install_id.to_string(),
            info,
        });
    }

    Ok(Json(GetInstallStatusesResp { installs }))
}

pub(in crate::api) async fn cancel_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .cancel_install(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::api) async fn list_install_events(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<Vec<InstallProgressEvent>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_events(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(in crate::api) async fn install_stream_sse(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, axum::Error>>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state
        .get_install_events(install_id)
        .await
        .unwrap_or_default();
    let initial = futures::stream::iter(history.into_iter().map(|ev| {
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        Ok::<_, axum::Error>(SseEvent::default().event("progress").data(payload))
    }));

    let live = futures::stream::unfold(sender.subscribe(), move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event("progress").data(payload)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = initial.chain(live);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
