use base64::Engine;
use ctx_core::ids::{RunId, SessionId};

pub(in crate::daemon::sessions::subagents) fn encode_agent_ref(session_id: SessionId) -> String {
    format!(
        "agent_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session_id.0.as_bytes())
    )
}

pub(super) fn decode_agent_ref(raw: &str) -> Result<SessionId, String> {
    let encoded = raw
        .trim()
        .strip_prefix("agent_")
        .ok_or_else(|| "invalid agent_id".to_string())?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "invalid agent_id".to_string())?;
    let uuid = uuid::Uuid::from_slice(&bytes).map_err(|_| "invalid agent_id".to_string())?;
    Ok(SessionId(uuid))
}

pub(in crate::daemon::sessions::subagents) fn encode_run_ref(run_id: RunId) -> String {
    format!(
        "run_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(run_id.0.as_bytes())
    )
}
