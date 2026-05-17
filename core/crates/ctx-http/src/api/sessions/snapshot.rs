use super::*;
#[path = "snapshot/events.rs"]
mod events;
#[path = "snapshot/head.rs"]
mod head;
#[path = "snapshot/history.rs"]
mod history;
#[path = "snapshot/state.rs"]
mod state;
#[path = "snapshot/vcs.rs"]
mod vcs;
pub(crate) use events::get_session_events;
pub(crate) use head::get_session_head;
pub(crate) use history::{get_session_history, list_session_turn_tools};
pub(crate) use state::get_session_state;
pub(crate) use vcs::{
    apply_session_diff_patch, get_session_diff, get_session_diff_summary, get_session_git_status,
};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSnapshotQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
}

pub(crate) async fn get_session_snapshot(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionSnapshotQuery>,
) -> Result<Json<ctx_core::models::SessionSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .load_session_snapshot(session_id, limit, include_events)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, String> {
    match raw {
        Some(value) => ctx_core::boolish::parse_boolish(value)
            .ok_or_else(|| format!("{label} must be one of: 1/true/yes/on or 0/false/no/off")),
        None => Ok(false),
    }
}

fn session_data_or_status<T>(result: anyhow::Result<Option<T>>) -> Result<T, StatusCode> {
    result
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
}
