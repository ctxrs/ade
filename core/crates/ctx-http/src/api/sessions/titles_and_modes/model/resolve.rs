use ctx_core::models::ExecutionEnvironment;
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::ProviderAdapter;

use super::*;
use crate::api::sessions::titles_and_modes::model::error::{
    internal_session_model_error, session_model_error, SessionModelResult,
};

pub(super) struct ResolvedSessionModelUpdate {
    pub(super) model_id: String,
    pub(super) reasoning_effort: Option<String>,
    pub(super) full_model_id: String,
}

pub(super) async fn ensure_session_model_adapter(
    state: &Arc<AppState>,
    session: &Session,
    install_target: InstallTarget,
) -> SessionModelResult<Arc<dyn ProviderAdapter>> {
    crate::daemon::installer::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await
    .map_err(internal_session_model_error)
}

pub(super) async fn resolve_session_model_update(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    session: &Session,
    execution_environment: ExecutionEnvironment,
    req: SetSessionModelReq,
) -> SessionModelResult<ResolvedSessionModelUpdate> {
    let reasoning_effort = req
        .reasoning_effort
        .as_deref()
        .map(normalize_effort_id)
        .filter(|value| !value.is_empty());
    if let Some(ref effort) = reasoning_effort {
        let allowed = ["none", "minimal", "low", "medium", "high", "xhigh"];
        if !allowed.contains(&effort.as_str()) {
            return Err(session_model_error(
                StatusCode::BAD_REQUEST,
                format!("unsupported reasoning effort '{effort}'"),
            ));
        }
    }

    let catalog = load_provider_model_catalog_for_execution_environment(
        state,
        workspace,
        &session.provider_id,
        execution_environment,
    )
    .await
    .map_err(internal_session_model_error)?;
    let resolved_model = resolve_model_id(
        Some(req.model_id.as_str()),
        reasoning_effort.as_deref(),
        None,
        catalog.as_ref(),
    )
    .map_err(|err| {
        session_model_error(
            StatusCode::BAD_REQUEST,
            logs::redact_sensitive(&err.to_string()),
        )
    })?;
    let full_model_id = compose_model_id(
        &resolved_model.model_id,
        resolved_model.reasoning_effort.as_deref(),
    );

    Ok(ResolvedSessionModelUpdate {
        model_id: resolved_model.model_id,
        reasoning_effort: resolved_model.reasoning_effort,
        full_model_id,
    })
}

pub(super) async fn switch_live_session_model(
    adapter: &dyn ProviderAdapter,
    session: &Session,
    full_model_id: &str,
) -> SessionModelResult<()> {
    let session_id = session.id.0.to_string();
    if adapter.has_live_session(&session_id).await {
        adapter
            .set_session_model(session_id, full_model_id.to_string())
            .await
            .map_err(|err| {
                session_model_error(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "failed to switch the live {} session to '{}': {}",
                        session.provider_id,
                        full_model_id,
                        logs::redact_sensitive(&err.to_string()),
                    ),
                )
            })?;
    }

    Ok(())
}
