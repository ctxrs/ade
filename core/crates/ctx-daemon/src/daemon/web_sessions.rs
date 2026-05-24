use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, NodeRuntimeSpec, WebSessionAccessError, WebSessionSignalBridgeError,
    WebSessionSignalUpstream, WebSessionSignalViewerGuard, WebSessionViewConnectPath,
    WebSessionViewPage, WorkerBundle,
};

use crate::daemon::{DaemonState, WebSessionRouteHandle};

mod launch;
mod route_contract;

pub use launch::{
    create_web_session, WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest,
};

pub struct PreparedWebSessionWorker {
    pub node_runtime: ctx_managed_installs::NodeRuntime,
    pub bundle: WorkerBundle,
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

impl WebSessionRouteHandle {
    pub async fn mint_web_session_view_connect_path(
        &self,
        id: &str,
    ) -> Result<WebSessionViewConnectPath, WebSessionAccessError> {
        self.web_sessions().mint_view_connect_path(id).await
    }

    pub async fn prepare_web_session_view_page(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<WebSessionViewPage, WebSessionAccessError> {
        self.web_sessions().prepare_view_page(id, token).await
    }

    pub async fn authorize_web_session_signal_bridge(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<(), WebSessionAccessError> {
        self.web_sessions().authorize_signal_access(id, token).await
    }

    pub async fn connect_web_session_signal_bridge(
        &self,
        session_id: String,
    ) -> Result<(WebSessionSignalUpstream, WebSessionSignalViewerGuard), WebSessionSignalBridgeError>
    {
        self.web_sessions_arc()
            .connect_signal_bridge(session_id)
            .await
    }
}
