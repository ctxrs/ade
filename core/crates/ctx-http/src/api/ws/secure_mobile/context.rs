use ctx_core::models::WorkspaceActiveSnapshotClientMessage;
use ctx_transport_runtime::mobile_e2ee;

use super::super::super::MobileSecureEnvelope;
use crate::daemon::mobile_access::MobileSecureStreamContext;

pub(super) fn decode_mobile_secure_client_message(
    context: &MobileSecureStreamContext,
    text: &str,
) -> Result<Option<WorkspaceActiveSnapshotClientMessage>, anyhow::Error> {
    let frame: MobileSecureEnvelope = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => return Ok(None),
    };
    let payload = mobile_e2ee::decrypt(
        &context.key,
        &context.device_id,
        frame.seq,
        &frame.nonce,
        &frame.ciphertext,
    )?;
    let message = serde_json::from_slice(&payload)?;
    Ok(Some(message))
}
