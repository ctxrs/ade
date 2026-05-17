use super::*;
use ctx_daemon::daemon::{CoreHandle, DaemonDiagnosticsSnapshot};

pub(in crate::api) async fn diagnostics(
    State(core): State<CoreHandle>,
) -> Result<Json<DaemonDiagnosticsSnapshot>, StatusCode> {
    let snapshot = core
        .diagnostics_snapshot(env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(snapshot))
}
