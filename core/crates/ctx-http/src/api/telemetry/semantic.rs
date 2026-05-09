use super::*;

mod request;
mod sanitizer;

use request::SemanticTelemetryBatch;

const MAX_SEMANTIC_EVENTS: usize = 200;

pub(in crate::api) async fn post_semantic_telemetry(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<SemanticTelemetryBatch>,
) -> Result<StatusCode, StatusCode> {
    if batch.event_count() > MAX_SEMANTIC_EVENTS {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let events = batch.into_events()?;
    state.telemetry.telemetry.emit_many(events).await;
    Ok(StatusCode::NO_CONTENT)
}
