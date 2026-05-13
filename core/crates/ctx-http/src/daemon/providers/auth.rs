use std::sync::Arc;

use chrono::Utc;
use ctx_core::models::Workspace;
use ctx_harness_sources::HarnessSourceKind;
use ctx_observability::logs;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_launch::probe;
use tokio::sync::mpsc;

use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::AppState;

pub(crate) enum ProviderWorkspaceAuthenticationError {
    ExecutionSettings(anyhow::Error),
    Verify(String),
}

pub(crate) struct ProviderWorkspaceAuthentication {
    pub(crate) install_target: InstallTarget,
    pub(crate) checked_at: String,
    pub(crate) error_message: Option<String>,
}

pub(crate) async fn authenticate_provider_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    method_id: Option<String>,
) -> Result<ProviderWorkspaceAuthentication, ProviderWorkspaceAuthenticationError> {
    let probe_context =
        probe::provider_auth_context_for_workspace_runtime(state.as_ref(), workspace, provider_id)
            .await
            .map_err(ProviderWorkspaceAuthenticationError::Verify)?;
    if probe_context.source.source_kind == HarnessSourceKind::Endpoint {
        return Err(ProviderWorkspaceAuthenticationError::Verify(
            "selected source is endpoint; update endpoint key/config directly instead of interactive authenticate"
                .to_string(),
        ));
    }

    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(ProviderWorkspaceAuthenticationError::ExecutionSettings)?;
    let (event_tx, mut event_rx) = mpsc::channel(32);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let checked_at = Utc::now().to_rfc3339();
    let result =
        match ctx_provider_runtime::provider_launch::resolver::ensure_provider_adapter_for_target(
            state.as_ref(),
            provider_id,
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

    let error_message = result
        .err()
        .map(|error| logs::redact_sensitive(&format!("{error:#}")));

    Ok(ProviderWorkspaceAuthentication {
        install_target,
        checked_at,
        error_message,
    })
}
