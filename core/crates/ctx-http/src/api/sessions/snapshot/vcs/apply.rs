use super::context::load_session_vcs_context;
use super::*;
use ctx_workspace_services::worktree_vcs::apply_worktree_vcs_session_patch;

pub(crate) async fn apply_session_diff_patch(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SessionDiffApplyReq>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    if req.patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    let action = req.action.trim().to_lowercase();
    let reverse = match action.as_str() {
        "accept" => false,
        "reject" => true,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "action must be accept or reject".to_string(),
                }),
            ));
        }
    };

    let ctx = load_session_vcs_context(&state, session_id).await?;
    apply_worktree_vcs_session_patch(
        std::path::Path::new(&ctx.worktree.root_path),
        &req.patch,
        reverse,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let resolution =
        resolve_session_diff_base(&state, &ctx.worktree, &SessionDiffQuery::default()).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff_apply", "no_target_branch", None)
            .await;
        return Ok(Json(SessionDiffResponse {
            diff: String::new(),
            available: false,
            unavailable_reason: Some(unavailable_reason),
        }));
    }
    let base_commit_sha = resolution.base_commit_sha;
    let diff = state
        .diff_worktree_for_session(&ctx.worktree, &base_commit_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(SessionDiffResponse {
        diff,
        available: true,
        unavailable_reason: None,
    }))
}
