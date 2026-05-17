use super::*;
use ctx_daemon::daemon::{SessionHeadRouteError, SessionHeadRouteRequest};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHeadQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
    pub(crate) min_event_seq: Option<i64>,
}

pub(crate) async fn get_session_head(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionHeadQuery>,
) -> Result<Json<SessionHeadSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = super::parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    state
        .session_head_for_route(SessionHeadRouteRequest {
            session_id,
            limit,
            include_events,
            min_event_seq: q.min_event_seq,
        })
        .await
        .map(Json)
        .map_err(session_head_route_status)
}

fn session_head_route_status(error: SessionHeadRouteError) -> StatusCode {
    match error {
        SessionHeadRouteError::BadRequest => StatusCode::BAD_REQUEST,
        SessionHeadRouteError::NotFound => StatusCode::NOT_FOUND,
        SessionHeadRouteError::Conflict => StatusCode::CONFLICT,
        SessionHeadRouteError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
