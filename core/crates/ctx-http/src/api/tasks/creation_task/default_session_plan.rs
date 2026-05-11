use super::*;
use target::resolve_default_session_target;

#[path = "default_session_plan/target.rs"]
mod target;

async fn validate_workspace_root_is_repo(
    workspace: &Workspace,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let workspace_root = StdPath::new(&workspace.root_path);
    let vcs = vcs::driver_for_path(workspace_root).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    vcs.assert_repo(workspace_root).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(())
}

pub(super) async fn preflight_default_session_creation(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
) -> Result<(ExecutionEnvironment, String, String, Option<String>), (StatusCode, Json<ApiErrorResp>)>
{
    validate_workspace_root_is_repo(workspace).await?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let execution_environment = execution_environment_from_settings(&effective);
    let (provider_id, model_id, reasoning_effort) =
        resolve_default_session_target(state, store, workspace, execution_environment).await?;
    Ok((
        execution_environment,
        provider_id,
        model_id,
        reasoning_effort,
    ))
}
