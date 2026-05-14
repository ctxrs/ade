use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, NodeRuntimeSpec, WebSessionInfo, WebSessionRunRequest,
    WebSessionRunResponse, WorkerBundle,
};

use crate::daemon::{DaemonState, TransportHandle};

mod access;
mod launch;
mod signal;

pub(crate) use access::{
    authorize_web_session_signal_access, mint_web_session_view_connect_path,
    prepare_web_session_view_page, WebSessionAccessError,
};
pub(crate) use launch::{
    create_web_session, WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest,
};
pub(crate) use signal::connect_web_session_signal_bridge;

pub(crate) struct PreparedWebSessionWorker {
    pub(crate) node_runtime: ctx_managed_installs::NodeRuntime,
    pub(crate) bundle: WorkerBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSessionActionError {
    NotFound,
    Internal,
}

pub(crate) async fn list_web_sessions(state: &Arc<DaemonState>) -> Vec<WebSessionInfo> {
    state.transport.web_sessions.list().await
}

pub(crate) async fn get_web_session(state: &Arc<DaemonState>, id: &str) -> Option<WebSessionInfo> {
    let handle = state.transport.web_sessions.get(id).await?;
    Some(handle.snapshot().await)
}

pub(crate) async fn run_web_session(
    state: &Arc<DaemonState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    match state.transport.web_sessions.run(id, payload).await {
        Ok(response) => Ok(response),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

pub(crate) async fn eval_web_session(
    state: &Arc<DaemonState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    match state.transport.web_sessions.eval(id, payload).await {
        Ok(response) => Ok(response),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

pub(crate) async fn close_web_session(
    state: &Arc<DaemonState>,
    id: &str,
) -> Result<(), WebSessionActionError> {
    match state.transport.web_sessions.close(id).await {
        Ok(()) => Ok(()),
        Err(_) => Err(classify_web_session_action_error(state, id).await),
    }
}

async fn classify_web_session_action_error(
    state: &Arc<DaemonState>,
    id: &str,
) -> WebSessionActionError {
    if state.transport.web_sessions.get(id).await.is_none() {
        WebSessionActionError::NotFound
    } else {
        WebSessionActionError::Internal
    }
}

pub(crate) async fn prepare_web_session_worker(
    state: &Arc<DaemonState>,
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

impl TransportHandle {
    pub(crate) async fn list_web_sessions(&self) -> Vec<WebSessionInfo> {
        list_web_sessions(&self.state).await
    }

    pub(crate) async fn get_web_session(&self, id: &str) -> Option<WebSessionInfo> {
        get_web_session(&self.state, id).await
    }

    pub(crate) async fn create_web_session(
        &self,
        request: WebSessionLaunchRequest,
    ) -> Result<WebSessionInfo, WebSessionLaunchError> {
        create_web_session(&self.state, request).await
    }

    pub(crate) async fn run_web_session(
        &self,
        id: &str,
        payload: WebSessionRunRequest,
    ) -> Result<WebSessionRunResponse, WebSessionActionError> {
        run_web_session(&self.state, id, payload).await
    }

    pub(crate) async fn eval_web_session(
        &self,
        id: &str,
        payload: WebSessionRunRequest,
    ) -> Result<WebSessionRunResponse, WebSessionActionError> {
        eval_web_session(&self.state, id, payload).await
    }

    pub(crate) async fn close_web_session(&self, id: &str) -> Result<(), WebSessionActionError> {
        close_web_session(&self.state, id).await
    }

    pub(crate) async fn mint_web_session_view_connect_path(
        &self,
        id: &str,
    ) -> Result<access::WebSessionViewConnectPath, WebSessionAccessError> {
        mint_web_session_view_connect_path(&self.state, id).await
    }

    pub(crate) async fn prepare_web_session_view_page(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<access::WebSessionViewPage, WebSessionAccessError> {
        prepare_web_session_view_page(&self.state, id, token).await
    }

    pub(crate) async fn authorize_web_session_signal_bridge(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<(), WebSessionAccessError> {
        authorize_web_session_signal_access(&self.state, id, token).await
    }

    pub(crate) async fn connect_web_session_signal_bridge(
        &self,
        session_id: String,
    ) -> Result<
        (
            signal::WebSessionSignalUpstream,
            signal::WebSessionSignalViewerGuard,
        ),
        signal::WebSessionSignalBridgeError,
    > {
        connect_web_session_signal_bridge(Arc::clone(&self.state), session_id).await
    }
}
