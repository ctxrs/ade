use super::*;

use ctx_provider_install::install_state::InstallTarget;
use ctx_transport_runtime::web_sessions::{ensure_worker_bundle, NodeRuntimeSpec, WorkerBundle};

pub(super) struct PreparedWebSessionWorker {
    pub(super) node_runtime: crate::daemon::installer::NodeRuntime,
    pub(super) bundle: WorkerBundle,
}

pub(super) async fn prepare_web_session_worker(
    state: &Arc<AppState>,
) -> Result<PreparedWebSessionWorker, WebSessionLaunchError> {
    let node_runtime = crate::daemon::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        InstallTarget::Host,
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare node runtime: {e}")))?;

    let bundle = ensure_worker_bundle(
        &state.core.data_root,
        &NodeRuntimeSpec {
            node_bin: node_runtime.node_bin.clone(),
            npm_cli_js: node_runtime.npm_cli_js.clone(),
        },
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare web session worker: {e}")))?;

    Ok(PreparedWebSessionWorker {
        node_runtime,
        bundle,
    })
}
