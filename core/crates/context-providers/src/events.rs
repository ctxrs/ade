use context_core::models::SessionEventType;

#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    pub event_type: SessionEventType,
    pub payload_json: serde_json::Value,
}
