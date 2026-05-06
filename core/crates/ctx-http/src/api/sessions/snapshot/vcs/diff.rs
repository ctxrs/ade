use super::context::load_session_vcs_context;
use super::*;
use crate::api::sessions::diff_exec::diff_worktree_for_session;
use crate::git_status::HttpWorktreeVcsSource;
use ctx_workspace_services::worktree_vcs::WorktreeVcsCommitLookupSource;

pub(crate) async fn get_session_diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(&state, session_id).await?;
    if !crate::git_status::worktree_has_vcs_repo(&state, &ctx.worktree)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?
    {
        return Ok(Json(SessionDiffResponse {
            diff: String::new(),
            available: false,
            unavailable_reason: Some(DiffUnavailableReason::NoRepo),
        }));
    }
    let resolution =
        resolve_session_diff_base(&state, &store, &ctx.workspace, &ctx.worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff", "no_target_branch", None)
            .await;
        return Ok(Json(SessionDiffResponse {
            diff: String::new(),
            available: false,
            unavailable_reason: Some(unavailable_reason),
        }));
    }
    let base_commit_sha = resolution.base_commit_sha;
    let diff = match diff_worktree_for_session(&state, &ctx.worktree, &base_commit_sha).await {
        Ok(diff) => diff,
        Err(err) if is_no_vcs_repo_error(&err) => {
            return Ok(Json(SessionDiffResponse {
                diff: String::new(),
                available: false,
                unavailable_reason: Some(DiffUnavailableReason::NoRepo),
            }));
        }
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            ));
        }
    };
    Ok(Json(SessionDiffResponse {
        diff,
        available: true,
        unavailable_reason: None,
    }))
}

pub(crate) async fn get_session_diff_summary(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffSummaryResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(&state, session_id).await?;
    if !crate::git_status::worktree_has_vcs_repo(&state, &ctx.worktree)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?
    {
        return Ok(Json(SessionDiffSummaryResponse {
            base_commit_sha: ctx.worktree.base_commit_sha.clone(),
            head_commit_sha: ctx.worktree.base_commit_sha.clone(),
            file_count: 0,
            line_additions: 0,
            line_deletions: 0,
            available: false,
            unavailable_reason: Some(DiffUnavailableReason::NoRepo),
        }));
    }
    let resolution =
        resolve_session_diff_base(&state, &store, &ctx.workspace, &ctx.worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff_summary", "no_target_branch", None)
            .await;
        let source = HttpWorktreeVcsSource::new(&state, &ctx.worktree);
        let head_commit_sha = source
            .resolve_commit("HEAD")
            .await
            .unwrap_or_else(|_| resolution.base_commit_sha.clone());
        return Ok(Json(SessionDiffSummaryResponse {
            base_commit_sha: resolution.base_commit_sha,
            head_commit_sha,
            file_count: 0,
            line_additions: 0,
            line_deletions: 0,
            available: false,
            unavailable_reason: Some(unavailable_reason),
        }));
    }
    let base_commit_sha = resolution.base_commit_sha;
    let (file_count, line_additions, line_deletions, available, unavailable_reason) =
        match diff_worktree_summary_for_session(&state, &ctx.worktree, &base_commit_sha).await {
            Ok((file_count, line_additions, line_deletions)) => {
                (file_count, line_additions, line_deletions, true, None)
            }
            Err(err) if is_no_vcs_repo_error(&err) => {
                (0, 0, 0, false, Some(DiffUnavailableReason::NoRepo))
            }
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&err.to_string()),
                    }),
                ));
            }
        };
    let source = HttpWorktreeVcsSource::new(&state, &ctx.worktree);
    let head_commit_sha = source
        .resolve_commit("HEAD")
        .await
        .unwrap_or_else(|_| base_commit_sha.clone());
    if available {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(ctx.worktree.id).await {
            if snapshot.compute_state == WorktreeVcsComputeState::Ready
                && snapshot.base_commit_sha == base_commit_sha
            {
                let summary = &snapshot.summary;
                let mismatch = summary.file_count != Some(file_count)
                    || summary.line_additions != Some(line_additions)
                    || summary.line_deletions != Some(line_deletions);
                if mismatch {
                    tracing::warn!(
                        worktree_id = %ctx.worktree.id.0,
                        snapshot_rev = snapshot.rev,
                        base_commit_sha = %base_commit_sha,
                        snapshot_file_count = ?summary.file_count,
                        snapshot_additions = ?summary.line_additions,
                        snapshot_deletions = ?summary.line_deletions,
                        summary_file_count = file_count,
                        summary_additions = line_additions,
                        summary_deletions = line_deletions,
                        "worktree vcs snapshot summary mismatch"
                    );
                }
            }
        }
    }
    Ok(Json(SessionDiffSummaryResponse {
        base_commit_sha,
        head_commit_sha,
        file_count,
        line_additions,
        line_deletions,
        available,
        unavailable_reason,
    }))
}
