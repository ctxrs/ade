use std::path::Path;

use anyhow::Result;

pub use ctx_transport_runtime::web_sessions::*;

pub(crate) async fn ensure_worker_bundle_for_node_runtime(
    data_root: &Path,
    node: &crate::installer::NodeRuntime,
) -> Result<WorkerBundle> {
    let node = NodeRuntimeSpec {
        node_bin: node.node_bin.clone(),
        npm_cli_js: node.npm_cli_js.clone(),
    };
    ctx_transport_runtime::ensure_worker_bundle(data_root, &node).await
}
