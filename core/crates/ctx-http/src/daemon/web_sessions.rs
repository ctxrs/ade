use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, NodeRuntimeSpec, WebSessionInfo, WebSessionRunRequest,
    WebSessionRunResponse, WorkerBundle,
};

use crate::daemon::AppState;

mod access;
mod launch;

pub(crate) use access::{
    authorize_web_session_signal_access, mint_web_session_view_connect_path,
    prepare_web_session_view_page, WebSessionAccessError,
};
pub(crate) use launch::{
    create_web_session, WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest,
};

pub(crate) struct PreparedWebSessionWorker {
    pub(crate) node_runtime: ctx_managed_installs::NodeRuntime,
    pub(crate) bundle: WorkerBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSessionActionError {
    NotFound,
    Internal,
}

pub(crate) async fn list_web_sessions(state: &Arc<AppState>) -> Vec<WebSessionInfo> {
    state.transport.web_sessions.list().await
}

pub(crate) async fn get_web_session(state: &Arc<AppState>, id: &str) -> Option<WebSessionInfo> {
    let handle = state.transport.web_sessions.get(id).await?;
    Some(handle.snapshot().await)
}

pub(crate) async fn run_web_session(
    state: &Arc<AppState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    match state.transport.web_sessions.run(id, payload).await {
        Ok(response) => Ok(response),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

pub(crate) async fn eval_web_session(
    state: &Arc<AppState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    match state.transport.web_sessions.eval(id, payload).await {
        Ok(response) => Ok(response),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

pub(crate) async fn close_web_session(
    state: &Arc<AppState>,
    id: &str,
) -> Result<(), WebSessionActionError> {
    match state.transport.web_sessions.close(id).await {
        Ok(()) => Ok(()),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

async fn classify_web_session_action_error(
    state: &Arc<AppState>,
    id: &str,
) -> WebSessionActionError {
    if state.transport.web_sessions.get(id).await.is_none() {
        WebSessionActionError::NotFound
    } else {
        WebSessionActionError::Internal
    }
}

pub(crate) async fn prepare_web_session_worker(
    state: &Arc<AppState>,
) -> anyhow::Result<PreparedWebSessionWorker> {
    let node_runtime = ctx_managed_installs::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        InstallTarget::Host,
    )
    .await
    .context("preparing node runtime for web session worker")?;

    let bundle = ensure_worker_bundle(
        &state.core.data_root,
        &NodeRuntimeSpec {
            node_bin: node_runtime.node_bin.clone(),
            npm_cli_js: node_runtime.npm_cli_js.clone(),
        },
    )
    .await
    .context("preparing web session worker bundle")?;

    Ok(PreparedWebSessionWorker {
        node_runtime,
        bundle,
    })
}
