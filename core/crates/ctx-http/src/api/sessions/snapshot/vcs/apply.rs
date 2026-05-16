use super::*;

pub(crate) async fn apply_session_diff_patch(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SessionDiffApplyReq>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = parse_session_id(&id)?;
    if req.patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    let action = parse_session_vcs_apply_action(&req.action)?;
    state
        .apply_session_vcs_diff_patch_for_request(session_id, action, &req.patch)
        .await
        .map(session_diff_response)
        .map(Json)
        .map_err(map_session_vcs_error)
}
