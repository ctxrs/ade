use super::*;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSnapshotQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHeadQuery {
    pub(crate) limit: Option<u32>,
    pub(crate) include_events: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionEventsQuery {
    pub(crate) after_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
    pub(crate) tail: Option<u32>,
    pub(crate) include_transient: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHistoryQuery {
    pub(crate) before_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
}

pub(crate) async fn get_session_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSnapshotQuery>,
) -> Result<Json<ctx_core::models::SessionSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = store_for_existing_session_status(&state, session_id).await?;
    match store
        .get_session_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(snapshot)) => Ok(Json(snapshot)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(crate) async fn get_session_head(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHeadQuery>,
) -> Result<Json<SessionHeadSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let workspace_id = match state
        .global_store()
        .get_workspace_id_for_session(session_id)
        .await
    {
        Ok(Some(workspace_id)) => workspace_id,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    if state.core.stores.is_workspace_deleting(workspace_id).await {
        return Err(StatusCode::NOT_FOUND);
    }
    if let Some(head) = state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_request(session_id, include_events, limit)
        .await
    {
        return Ok(Json(head));
    }
    state.emit_cache_miss("session_head").await;
    let store = store_for_existing_session_status(&state, session_id).await?;
    match store
        .get_session_head_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(head)) => {
            state.emit_cache_rehydrate("session_head", true).await;
            if include_events {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .update_session_head(head.clone())
                    .await;
            } else {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .update_compact_session_head(head.clone())
                    .await;
            }
            Ok(Json(head))
        }
        Ok(None) => {
            state.emit_cache_rehydrate("session_head", false).await;
            Err(StatusCode::NOT_FOUND)
        }
        Err(_) => {
            state.emit_cache_rehydrate("session_head", false).await;
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub(crate) async fn get_session_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if session.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut state = store
        .get_session_state(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for artifact in state.artifacts.iter_mut() {
        if tokio::fs::metadata(&artifact.absolute_path).await.is_err() {
            artifact.missing = Some(true);
        }
    }
    Ok(Json(state))
}

pub(crate) async fn get_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionEventsQuery>,
) -> Result<Json<ctx_core::models::SessionEventsPage>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 200;
    const MAX_LIMIT: u32 = 1000;

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let include_transient = parse_boolish_flag(q.include_transient.as_deref(), "include_transient")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = store_for_existing_session_status(&state, session_id).await?;

    let (events, has_more, next_cursor) = if let Some(tail) = q.tail {
        let tail = tail.clamp(1, MAX_LIMIT);
        let mut rows = store
            .list_session_events_tail_by_seq(session_id, tail + 1, include_transient)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > tail;
        if has_more {
            rows = rows.split_off(rows.len().saturating_sub(tail as usize));
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    } else {
        let mut rows = store
            .list_session_events_page_by_seq(
                session_id,
                q.after_seq,
                Some(limit + 1),
                include_transient,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > limit;
        if has_more {
            rows.truncate(limit as usize);
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    };

    Ok(Json(ctx_core::models::SessionEventsPage {
        session_id,
        events,
        next_cursor,
        has_more,
    }))
}

pub(crate) async fn get_session_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHistoryQuery>,
) -> Result<Json<SessionHistoryPage>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let store = store_for_existing_session_status(&state, session_id).await?;
    match store
        .get_session_history_page(session_id, q.before_seq, limit)
        .await
    {
        Ok(Some(page)) => Ok(Json(page)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(crate) async fn list_session_turn_tools(
    State(state): State<Arc<AppState>>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<Vec<SessionTurnTool>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    store
        .list_turn_tools(session_id, turn_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn is_true(v: &bool) -> bool {
    *v
}

pub(crate) fn is_no_vcs_repo_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    lower.contains("no vcs repo found")
        || lower.contains("not a git repository")
        || lower.contains("is not a git repo")
        || lower.contains("not inside a jj repo")
}

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
    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            )
        })?;
    let resolution = resolve_session_diff_base(&state, &store, &workspace, &worktree, &q).await?;
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
    let diff = match diff_worktree_for_session(&state, &worktree, &base_commit_sha).await {
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
    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            )
        })?;
    let resolution = resolve_session_diff_base(&state, &store, &workspace, &worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff_summary", "no_target_branch", None)
            .await;
        let head_commit_sha = crate::git_status::worktree_rev_parse_head(&state, &worktree)
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
        match diff_worktree_summary_for_session(&state, &worktree, &base_commit_sha).await {
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
    let head_commit_sha = crate::git_status::worktree_rev_parse_head(&state, &worktree)
        .await
        .unwrap_or_else(|_| base_commit_sha.clone());
    if available {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
            if snapshot.compute_state == WorktreeVcsComputeState::Ready
                && snapshot.base_commit_sha == base_commit_sha
            {
                let summary = &snapshot.summary;
                let mismatch = summary.file_count != Some(file_count)
                    || summary.line_additions != Some(line_additions)
                    || summary.line_deletions != Some(line_deletions);
                if mismatch {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
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
    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let snapshot = load_git_status_snapshot(&state, &worktree)
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
        .upsert_session_git_status_summary(session_id, worktree.id, &summary)
        .await
    {
        tracing::warn!(session_id = %session_id.0, "git status summary persist failed: {err:?}");
    }
    Ok(Json(resp))
}

pub(crate) struct WorktreeDiffBaseResolution {
    pub base_commit_sha: String,
    pub target_branch: Option<String>,
    pub target_source: Option<WorktreeVcsTargetSource>,
    pub kind: WorktreeVcsBaseResolutionKind,
    pub error: Option<String>,
    pub unavailable_reason: Option<DiffUnavailableReason>,
    pub explicit_target: bool,
}

pub(crate) async fn resolve_diff_base_with_meta(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> WorktreeDiffBaseResolution {
    if let Some(base) = query.base_commit_sha.as_deref() {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return WorktreeDiffBaseResolution {
                base_commit_sha: trimmed.to_string(),
                target_branch: None,
                target_source: None,
                kind: WorktreeVcsBaseResolutionKind::ExplicitBase,
                error: None,
                unavailable_reason: None,
                explicit_target: false,
            };
        }
    }

    let explicit_target = query
        .target_branch
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let mut target_branch = match query.target_branch.as_deref() {
        Some(target) if !target.trim().is_empty() => Some(target.trim().to_string()),
        _ => None,
    };
    let mut target_source = if target_branch.is_some() {
        Some(WorktreeVcsTargetSource::Explicit)
    } else {
        None
    };

    if target_branch.is_none() {
        match workspace_config::load_primary_branch(store).await {
            Ok(Some(branch)) => {
                target_branch = Some(branch);
                target_source = Some(WorktreeVcsTargetSource::PrimaryBranchConfig);
            }
            Ok(None) => {
                return WorktreeDiffBaseResolution {
                    base_commit_sha: worktree.base_commit_sha.clone(),
                    target_branch: None,
                    target_source: None,
                    kind: WorktreeVcsBaseResolutionKind::WorktreeBase,
                    error: Some("workspace primary branch is not configured".to_string()),
                    unavailable_reason: Some(DiffUnavailableReason::NoTargetBranch),
                    explicit_target,
                };
            }
            Err(err) => tracing::warn!(
                workspace_id = %workspace.id.0,
                "failed to load workspace primary branch: {err:#}"
            ),
        }
    }

    let mut error: Option<String> = None;
    let mut unavailable_reason: Option<DiffUnavailableReason> = None;
    if let Some(target_branch) = target_branch.clone() {
        match crate::git_status::worktree_merge_base(state, worktree, &target_branch).await {
            Ok(base) => {
                return WorktreeDiffBaseResolution {
                    base_commit_sha: base,
                    target_branch: Some(target_branch),
                    target_source,
                    kind: WorktreeVcsBaseResolutionKind::MergeBase,
                    error: None,
                    unavailable_reason: None,
                    explicit_target,
                };
            }
            Err(err) => {
                let redacted = logs::redact_sensitive(&err.to_string());
                error = Some(redacted);
                unavailable_reason = Some(DiffUnavailableReason::NoTargetBranch);
                tracing::warn!(
                    worktree_id = %worktree.id.0,
                    "merge-base failed for target {target_branch}: {err:#}"
                );
            }
        }
    }

    WorktreeDiffBaseResolution {
        base_commit_sha: worktree.base_commit_sha.clone(),
        target_branch,
        target_source,
        kind: WorktreeVcsBaseResolutionKind::WorktreeBase,
        error,
        unavailable_reason,
        explicit_target,
    }
}

pub(crate) async fn resolve_session_diff_base(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> Result<WorktreeDiffBaseResolution, (StatusCode, Json<ApiErrorResp>)> {
    let resolution = resolve_diff_base_with_meta(state, store, workspace, worktree, query).await;
    if resolution.explicit_target {
        if let Some(error) = resolution.error.clone() {
            return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })));
        }
    }
    Ok(resolution)
}

pub(crate) async fn apply_session_diff_patch(
    State(state): State<Arc<AppState>>,
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

    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            )
        })?;

    ctx_fs::git::git_apply_patch(
        &worktree.root_path,
        &req.patch,
        ctx_fs::git::ApplyPatchTarget::Worktree,
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

    let resolution = resolve_session_diff_base(
        &state,
        &store,
        &workspace,
        &worktree,
        &SessionDiffQuery::default(),
    )
    .await?;
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
    let diff = diff_worktree_for_session(&state, &worktree, &base_commit_sha)
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

#[derive(Debug, Deserialize)]
pub(crate) struct SessionDiffApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffResponse {
    diff: String,
    #[serde(skip_serializing_if = "is_true")]
    available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionDiffQuery {
    pub(crate) base_commit_sha: Option<String>,
    pub(crate) target_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffSummaryResponse {
    base_commit_sha: String,
    head_commit_sha: String,
    file_count: i64,
    line_additions: i64,
    line_deletions: i64,
    #[serde(skip_serializing_if = "is_true")]
    available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionGitStatusResponse {
    raw: String,
    summary_line: String,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: i64,
    behind: i64,
    detached: bool,
    staged: i64,
    unstaged: i64,
    untracked: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<GitStatusEntry>,
    entries_truncated: bool,
    entries_total_count: i64,
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, String> {
    match raw {
        Some(value) => ctx_core::boolish::parse_boolish(value)
            .ok_or_else(|| format!("{label} must be one of: 1/true/yes/on or 0/false/no/off")),
        None => Ok(false),
    }
}
