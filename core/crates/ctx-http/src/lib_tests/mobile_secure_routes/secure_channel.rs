use ctx_core::ids::WorkspaceId;

pub(super) fn mobile_secure_stream_query(
    device_id: &str,
    key: &ctx_transport_runtime::mobile_e2ee::E2eeKey,
    workspace_id: WorkspaceId,
) -> String {
    let token =
        ctx_transport_runtime::mobile_e2ee::derive_stream_token(key, &workspace_id.0.to_string());
    format!("device_id={device_id}&token={token}")
}
