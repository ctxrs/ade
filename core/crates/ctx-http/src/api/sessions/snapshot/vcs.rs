use super::*;

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
    if !crate::git_status::worktree_has_vcs_repo(&state, &worktree)
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
    if !crate::git_status::worktree_has_vcs_repo(&state, &worktree)
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
            base_commit_sha: worktree.base_commit_sha.clone(),
            head_commit_sha: worktree.base_commit_sha.clone(),
            file_count: 0,
            line_additions: 0,
            line_deletions: 0,
            available: false,
            unavailable_reason: Some(DiffUnavailableReason::NoRepo),
        }));
    }
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
    pub(crate) action: String,
    pub(crate) patch: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffResponse {
    pub(crate) diff: String,
    #[serde(skip_serializing_if = "is_true")]
    pub(crate) available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionDiffQuery {
    pub(crate) base_commit_sha: Option<String>,
    pub(crate) target_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionDiffSummaryResponse {
    pub(crate) base_commit_sha: String,
    pub(crate) head_commit_sha: String,
    pub(crate) file_count: i64,
    pub(crate) line_additions: i64,
    pub(crate) line_deletions: i64,
    #[serde(skip_serializing_if = "is_true")]
    pub(crate) available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<DiffUnavailableReason>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionGitStatusResponse {
    pub(crate) raw: String,
    pub(crate) summary_line: String,
    pub(crate) branch: Option<String>,
    pub(crate) upstream: Option<String>,
    pub(crate) ahead: i64,
    pub(crate) behind: i64,
    pub(crate) detached: bool,
    pub(crate) staged: i64,
    pub(crate) unstaged: i64,
    pub(crate) untracked: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) entries: Vec<GitStatusEntry>,
    pub(crate) entries_truncated: bool,
    pub(crate) entries_total_count: i64,
}
