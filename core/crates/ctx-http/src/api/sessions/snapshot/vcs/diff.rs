use super::*;

pub(crate) async fn get_session_diff(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffRouteQuery>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = parse_session_id(&id)?;
    state
        .get_session_vcs_diff_for_request(session_id, session_diff_query(q))
        .await
        .map(session_diff_response)
        .map(Json)
        .map_err(map_session_vcs_error)
}

pub(crate) async fn get_session_diff_summary(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffRouteQuery>,
) -> Result<Json<SessionDiffSummaryResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = parse_session_id(&id)?;
    state
        .get_session_vcs_diff_summary_for_request(session_id, session_diff_query(q))
        .await
        .map(session_diff_summary_response)
        .map(Json)
        .map_err(map_session_vcs_error)
}
