use super::*;
use crate::daemon::installer::ensure_provider_adapter_for_target;

mod verify;

pub(in crate::api) use verify::verify_provider_for_workspace;

pub(in crate::api) async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    req: Option<Json<AuthenticateProviderReq>>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    let method_id = req.and_then(|value| value.0.method_id);
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))?;

    let probe_context = probe::provider_auth_context_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": err,
            })),
        )
    })?;
    if probe_context.source.source_kind == HarnessSourceKind::Endpoint {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "selected source is endpoint; update endpoint key/config directly instead of interactive authenticate",
            })),
        ));
    }

    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let checked_at = Utc::now().to_rfc3339();
    let result = match ensure_provider_adapter_for_target(
        state.as_ref(),
        &provider_id,
        install_target,
    )
    .await
    {
        Ok(adapter) => {
            adapter
                .authenticate_session(
                    format!("auth-{}", uuid::Uuid::new_v4()),
                    probe_context.cwd,
                    probe_context.env,
                    method_id,
                    event_tx,
                    ctx_providers::adapters::ProviderRunHooks::default(),
                )
                .await
        }
        Err(err) => Err(err),
    };

    let resp = match result {
        Ok(()) => ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "ok".to_string(),
            auth_required: Some(false),
            checked_at: Some(checked_at),
            message: None,
        },
        Err(err) => {
            let msg = logs::redact_sensitive(&err.to_string());
            let (status, auth_required, _) = classify_probe_error(&msg);
            ProviderAuthCheckResp {
                provider_id: provider_id.clone(),
                workspace_id: ws_id.0.to_string(),
                status: status.to_string(),
                auth_required,
                checked_at: Some(checked_at),
                message: Some(msg),
            }
        }
    };
    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
    state
        .providers
        .with_provider_verify_cache(|cache| {
            cache.insert(
                cache_key,
                crate::daemon::CachedProviderVerify {
                    cached_at: std::time::Instant::now(),
                    value: verify_value,
                },
            );
        })
        .await;

    Ok(Json(resp))
}
