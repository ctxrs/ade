use super::context::load_session_vcs_context;
use super::*;
use crate::git_status::load_git_status_snapshot;

pub(crate) async fn get_session_git_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionGitStatusResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(&state, session_id).await?;
    let snapshot = load_git_status_snapshot(&state, &ctx.worktree, true, true)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let resp = SessionGitStatusResponse {
        raw: snapshot.raw,
        summary_line: snapshot.summary_line,
        branch: snapshot.branch,
        upstream: snapshot.upstream,
        ahead: snapshot.ahead,
        behind: snapshot.behind,
        detached: snapshot.detached,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked: snapshot.untracked,
        entries: snapshot.entries,
        entries_truncated: snapshot.entries_truncated,
        entries_total_count: snapshot.entries_total_count,
    };
    let summary = SessionGitStatusSummary {
        summary_line: resp.summary_line.clone(),
        branch: resp.branch.clone(),
        upstream: resp.upstream.clone(),
        ahead: resp.ahead,
        behind: resp.behind,
        detached: resp.detached,
        staged: resp.staged,
        unstaged: resp.unstaged,
        untracked: resp.untracked,
    };
    if let Err(err) = store
        .upsert_session_git_status_summary(ctx.session.id, ctx.worktree.id, &summary)
        .await
    {
        tracing::warn!(session_id = %ctx.session.id.0, "git status summary persist failed: {err:?}");
    }
    Ok(Json(resp))
}
