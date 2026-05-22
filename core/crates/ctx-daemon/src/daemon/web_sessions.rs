use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, NodeRuntimeSpec, WebSessionAccessError, WebSessionInfo,
    WebSessionRunRequest, WebSessionRunResponse, WorkerBundle,
};

use crate::daemon::{DaemonState, TransportHandle};

mod launch;
mod route_contract;

pub use ctx_transport_runtime::web_sessions::{
    WebSessionActionError, WebSessionSignalBridgeError, WebSessionSignalUpstream,
    WebSessionSignalViewerGuard, WebSessionViewConnectPath, WebSessionViewPage,
};
pub use launch::{
    create_web_session, WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest,
};
pub use route_contract::{
    WebSessionActionRouteRequest, WebSessionCreateRouteRequest, WebSessionListRouteQuery,
    WebSessionRouteError, WebSessionRouteErrorKind,
};

pub struct PreparedWebSessionWorker {
    pub node_runtime: ctx_managed_installs::NodeRuntime,
    pub bundle: WorkerBundle,
}

pub async fn list_web_sessions(state: &Arc<DaemonState>) -> Vec<WebSessionInfo> {
    state.transport.web_sessions.list().await
}

pub async fn get_web_session(state: &Arc<DaemonState>, id: &str) -> Option<WebSessionInfo> {
    state.transport.web_sessions.get_info(id).await
}

pub async fn run_web_session(
    state: &Arc<DaemonState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    state.transport.web_sessions.run_action(id, payload).await
}

pub async fn eval_web_session(
    state: &Arc<DaemonState>,
    id: &str,
    payload: WebSessionRunRequest,
) -> Result<WebSessionRunResponse, WebSessionActionError> {
    state.transport.web_sessions.eval_action(id, payload).await
}

pub async fn close_web_session(
    state: &Arc<DaemonState>,
    id: &str,
) -> Result<(), WebSessionActionError> {
    state.transport.web_sessions.close_action(id).await
}

pub async fn prepare_web_session_worker(
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
    pub async fn list_web_sessions(&self) -> Vec<WebSessionInfo> {
        list_web_sessions(&self.state).await
    }

    pub async fn get_web_session(&self, id: &str) -> Option<WebSessionInfo> {
        get_web_session(&self.state, id).await
    }

    pub async fn create_web_session(
        &self,
        request: WebSessionLaunchRequest,
    ) -> Result<WebSessionInfo, WebSessionLaunchError> {
        create_web_session(&self.state, request).await
    }

    pub async fn run_web_session(
        &self,
        id: &str,
        payload: WebSessionRunRequest,
    ) -> Result<WebSessionRunResponse, WebSessionActionError> {
        run_web_session(&self.state, id, payload).await
    }

    pub async fn eval_web_session(
        &self,
        id: &str,
        payload: WebSessionRunRequest,
    ) -> Result<WebSessionRunResponse, WebSessionActionError> {
        eval_web_session(&self.state, id, payload).await
    }

    pub async fn close_web_session(&self, id: &str) -> Result<(), WebSessionActionError> {
        close_web_session(&self.state, id).await
    }

    pub async fn mint_web_session_view_connect_path(
        &self,
        id: &str,
    ) -> Result<WebSessionViewConnectPath, WebSessionAccessError> {
        self.state
            .transport
            .web_sessions
            .mint_view_connect_path(id)
            .await
    }

    pub async fn prepare_web_session_view_page(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<WebSessionViewPage, WebSessionAccessError> {
        self.state
            .transport
            .web_sessions
            .prepare_view_page(id, token)
            .await
    }

    pub async fn authorize_web_session_signal_bridge(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<(), WebSessionAccessError> {
        self.state
            .transport
            .web_sessions
            .authorize_signal_access(id, token)
            .await
    }

    pub async fn connect_web_session_signal_bridge(
        &self,
        session_id: String,
    ) -> Result<(WebSessionSignalUpstream, WebSessionSignalViewerGuard), WebSessionSignalBridgeError>
    {
        self.state
            .transport
            .web_sessions
            .clone()
            .connect_signal_bridge(session_id)
            .await
    }
}
