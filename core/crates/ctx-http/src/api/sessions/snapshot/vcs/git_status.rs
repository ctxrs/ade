use super::*;

pub(crate) async fn get_session_git_status(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<SessionGitStatusResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = parse_session_id(&id)?;
    state
        .get_session_vcs_git_status_for_request(session_id)
        .await
        .map(session_git_status_response)
        .map(Json)
        .map_err(map_session_vcs_error)
}
