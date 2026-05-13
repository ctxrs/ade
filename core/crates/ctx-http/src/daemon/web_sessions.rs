use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{ensure_worker_bundle, NodeRuntimeSpec, WorkerBundle};

use crate::daemon::AppState;

pub(crate) struct PreparedWebSessionWorker {
    pub(crate) node_runtime: ctx_managed_installs::NodeRuntime,
    pub(crate) bundle: WorkerBundle,
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
