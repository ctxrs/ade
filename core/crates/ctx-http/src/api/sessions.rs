use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::Engine;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::artifacts::persist_blob_bytes;
use super::errors::ApiErrorResp;
use super::extractors::extract_model_entries;
use super::providers::default_agent_server_command;
use super::redact_json_value;
use super::shared::{
    env_target_for_worktree, load_and_cache_worktree_files, FileCompletionsQuery, SessionWithEnv,
};
use crate::attachments;
use crate::completions;
use crate::daemon::{AppState, GitStatusSnapshotCacheEntry};
use crate::git_status::{load_git_status_snapshot, GitStatusEntry};
use crate::installer;
use crate::logs;
use crate::oracle;
use crate::provider_accounts;
use crate::scheduler::SchedulerCommand;
use crate::settings as user_settings;
use crate::title_generation;
use crate::workspace_config;
use crate::worktree_bootstrap;
use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_fs::git::git_merge_base;
use ctx_fs::vcs;
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};
use ctx_providers::events::NormalizedEvent;
use ctx_providers::{
    acp::{probe_provider_options, AcpAgentConfig, AcpClientConfig},
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
    crp::probe_crp_models,
};
use tokio::sync::mpsc;
pub(super) async fn get_session_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSnapshotQuery>,
) -> Result<Json<SessionSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match store
        .get_session_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(snapshot)) => Ok(Json(snapshot)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(super) async fn get_session_head(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHeadQuery>,
) -> Result<Json<SessionHeadSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if let Some(mut head) = state
        .workspace_active_snapshot
        .get_session_head(session_id)
        .await
    {
        if !include_events {
            head.events.clear();
            head.head_window.event_count = 0;
        }
        return Ok(Json(head));
    }
    state.emit_cache_miss("session_head").await;
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match store
        .get_session_head_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(head)) => {
            state.emit_cache_rehydrate("session_head", true).await;
            if include_events {
                state
                    .workspace_active_snapshot
                    .update_session_head(head.clone())
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

pub(super) async fn get_session_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
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

pub(super) async fn get_session_diff(
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
    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
    let base_commit_sha = resolve_session_diff_base(&workspace, &worktree, &q).await?;
    let diff = ctx_fs::worktrees::diff_worktree(&worktree.root_path, &base_commit_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(SessionDiffResponse { diff }))
}

pub(super) async fn get_session_diff_summary(
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
    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
    let base_commit_sha = resolve_session_diff_base(&workspace, &worktree, &q).await?;
    let (file_count, line_additions, line_deletions) =
        ctx_fs::worktrees::diff_worktree_summary(&worktree.root_path, &base_commit_sha)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    let head_commit_sha = match vcs::driver_for_path(StdPath::new(&worktree.root_path)).await {
        Ok(vcs) => match vcs.rev_parse_head(StdPath::new(&worktree.root_path)).await {
            Ok(value) => value,
            Err(_) => base_commit_sha.clone(),
        },
        Err(_) => base_commit_sha.clone(),
    };
    Ok(Json(SessionDiffSummaryResponse {
        base_commit_sha,
        head_commit_sha,
        file_count,
        line_additions,
        line_deletions,
    }))
}

pub(super) async fn get_session_git_status(
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
    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
    let snapshot = load_git_status_snapshot(&worktree).await.map_err(|e| {
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
    maybe_emit_git_status_snapshot(&state, session_id, worktree.id, &resp).await;
    Ok(Json(resp))
}

pub(super) async fn resolve_session_diff_base(
    workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> Result<String, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(base) = query.base_commit_sha.as_deref() {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let explicit_target = query
        .target_branch
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let target_branch = match query.target_branch.as_deref() {
        Some(target) if !target.trim().is_empty() => Some(target.trim().to_string()),
        _ => match workspace_config::load_merge_queue_config(StdPath::new(&workspace.root_path))
            .await
        {
            Ok(cfg) => Some(cfg.target_branch),
            Err(err) => {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    "failed to load merge queue config: {err:#}"
                );
                None
            }
        },
    };
    if let Some(target_branch) = target_branch {
        match git_merge_base(&worktree.root_path, &target_branch, "HEAD").await {
            Ok(base) => return Ok(base),
            Err(err) => {
                if explicit_target {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&err.to_string()),
                        }),
                    ));
                }
                tracing::warn!(
                    worktree_id = %worktree.id.0,
                    "merge-base failed for target {target_branch}: {err:#}"
                );
            }
        }
    }
    Ok(worktree.base_commit_sha.clone())
}

pub(super) async fn maybe_emit_git_status_snapshot(
    state: &Arc<AppState>,
    session_id: SessionId,
    worktree_id: WorktreeId,
    snapshot: &SessionGitStatusResponse,
) {
    const GIT_STATUS_DEBOUNCE_MS: u64 = 500;
    const GIT_STATUS_MAX_INTERVAL_MS: u64 = 2000;
    let summary = serde_json::json!({
        "summary_line": snapshot.summary_line,
        "branch": snapshot.branch,
        "upstream": snapshot.upstream,
        "ahead": snapshot.ahead,
        "behind": snapshot.behind,
        "detached": snapshot.detached,
        "staged": snapshot.staged,
        "unstaged": snapshot.unstaged,
        "untracked": snapshot.untracked,
    });
    let payload = serde_json::json!({
        "kind": "git_status_snapshot",
        "worktree_id": worktree_id.0.to_string(),
        "summary": summary,
        "entries": snapshot.entries,
    });
    let payload_raw = match serde_json::to_string(&payload) {
        Ok(value) => value,
        Err(_) => return,
    };
    let now = Instant::now();
    {
        let mut cache = state.git_status_snapshots.lock().await;
        let entry = cache.entry(worktree_id).or_insert_with(|| {
            crate::daemon::TimedEntry::new(GitStatusSnapshotCacheEntry {
                payload: String::new(),
                emitted_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
                last_change_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
            })
        });
        entry.touch_at(now);
        let is_first = entry.value.payload.is_empty();
        if entry.value.payload == payload_raw {
            return;
        }
        let since_change = now.duration_since(entry.value.last_change_at);
        entry.value.payload = payload_raw;
        entry.value.last_change_at = now;
        let since_emit = now.duration_since(entry.value.emitted_at);
        if !is_first
            && since_emit < Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS)
            && since_change < Duration::from_millis(GIT_STATUS_DEBOUNCE_MS)
        {
            return;
        }
        entry.value.emitted_at = now;
    }
    let notice = match state.store_for_session(session_id).await {
        Ok(store) => {
            store
                .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
                .await
        }
        Err(_) => return,
    };
    if let Ok(event) = notice {
        state.publish_event(event).await;
    }
}

pub(super) async fn apply_session_diff_patch(
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

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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

    let base_commit_sha =
        resolve_session_diff_base(&workspace, &worktree, &SessionDiffQuery::default()).await?;
    let diff = ctx_fs::worktrees::diff_worktree(&worktree.root_path, &base_commit_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(SessionDiffResponse { diff }))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TitleGenerationSource {
    Llm,
    Fallback,
}

impl TitleGenerationSource {
    fn as_str(self) -> &'static str {
        match self {
            TitleGenerationSource::Llm => "llm",
            TitleGenerationSource::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TitleGenerationOutcome {
    title: String,
    source: TitleGenerationSource,
}

pub(super) async fn configured_title_generation_settings(
    state: &AppState,
) -> Option<user_settings::TitleGenerationSettings> {
    let settings = user_settings::load_settings(&state.data_root).await;
    settings
        .title_generation
        .as_ref()
        .filter(|cfg| title_generation::is_configured(cfg))
        .cloned()
}

pub(super) async fn generate_title_for_prompt(
    cfg: Option<&user_settings::TitleGenerationSettings>,
    prompt: &str,
    data_root: &StdPath,
) -> anyhow::Result<TitleGenerationOutcome> {
    let fallback = title_generation::fallback_title_from_prompt(prompt);
    if fallback.trim().is_empty() {
        return Err(anyhow::anyhow!("prompt is empty"));
    }

    if let Some(cfg) = cfg.filter(|c| title_generation::is_configured(c)) {
        match title_generation::generate_title(cfg, prompt, data_root).await {
            Ok(title) => {
                return Ok(TitleGenerationOutcome {
                    title,
                    source: TitleGenerationSource::Llm,
                })
            }
            Err(err) => {
                tracing::warn!(
                    "title generation failed: {}",
                    logs::redact_sensitive(&err.to_string())
                );
                // TODO: surface a snackbar/toast when title generation fails.
            }
        }
    }

    Ok(TitleGenerationOutcome {
        title: fallback,
        source: TitleGenerationSource::Fallback,
    })
}

pub(super) async fn apply_session_title_update(
    state: &Arc<AppState>,
    session: &Session,
    outcome: TitleGenerationOutcome,
) -> anyhow::Result<()> {
    let store = state.store_for_session(session.id).await?;
    let updated = store
        .update_session_title(session.id, outcome.title.clone())
        .await
        .context("updating session title")?;
    if !updated {
        return Ok(());
    }

    if let Ok(Some(updated_session)) = store.get_session(session.id).await {
        state.remember_session_meta(&updated_session).await;
    }

    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    let mut task_updated = false;
    if let Ok(Some(task)) = store.get_task(session.task_id).await {
        let title = task.title.trim();
        if (title.is_empty() || title == title_generation::DEFAULT_SESSION_TITLE)
            && store
                .update_task_title(session.task_id, outcome.title.clone())
                .await
                .unwrap_or(false)
        {
            task_updated = true;
        }
    }

    if task_updated {
        if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
            tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
        }
    }

    let notice = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "title_generated",
                "title": outcome.title,
                "source": outcome.source.as_str(),
            }),
        )
        .await;
    if let Ok(event) = notice {
        state.publish_event(event).await;
    }

    Ok(())
}

pub(super) async fn maybe_generate_session_title(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
    cfg: Option<user_settings::TitleGenerationSettings>,
) -> anyhow::Result<Option<TitleGenerationOutcome>> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Ok(None);
    }

    let current = session.title.trim();
    if !force && !current.is_empty() && current != title_generation::DEFAULT_SESSION_TITLE {
        return Ok(None);
    }

    let outcome = generate_title_for_prompt(cfg.as_ref(), &prompt, &state.data_root).await?;
    apply_session_title_update(&state, &session, outcome.clone()).await?;
    Ok(Some(outcome))
}

pub(super) async fn schedule_session_title_generation(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
) -> bool {
    let cfg = configured_title_generation_settings(&state).await;
    if cfg.is_some() {
        tokio::spawn(async move {
            let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        });
        true
    } else {
        let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        false
    }
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionSnapshotQuery {
    limit: Option<u32>,
    include_events: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionHeadQuery {
    limit: Option<u32>,
    include_events: Option<String>,
}
#[derive(Debug, Deserialize)]
pub(super) struct SessionDiffApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}
#[derive(Debug, Serialize)]
pub(super) struct SessionDiffResponse {
    diff: String,
}
#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionDiffQuery {
    base_commit_sha: Option<String>,
    target_branch: Option<String>,
}
#[derive(Debug, Serialize)]
pub(super) struct SessionDiffSummaryResponse {
    base_commit_sha: String,
    head_commit_sha: String,
    file_count: i64,
    line_additions: i64,
    line_deletions: i64,
}
#[derive(Debug, Serialize)]
pub(super) struct SessionGitStatusResponse {
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
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionEventsQuery {
    after_seq: Option<i64>,
    limit: Option<u32>,
    tail: Option<u32>,
    include_transient: Option<String>,
}

pub(super) async fn get_session_events(
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
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

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

#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionHistoryQuery {
    before_seq: Option<i64>,
    limit: Option<u32>,
}

pub(super) async fn get_session_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHistoryQuery>,
) -> Result<Json<SessionHistoryPage>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match store
        .get_session_history_page(session_id, q.before_seq, limit)
        .await
    {
        Ok(Some(page)) => Ok(Json(page)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(super) async fn list_session_turn_tools(
    State(state): State<Arc<AppState>>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<Vec<SessionTurnTool>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    store
        .list_turn_tools(session_id, turn_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(super) async fn session_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let query = q.query.unwrap_or_default();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

    let files = {
        let now = Instant::now();
        let mut cache = state.file_completions_cache.lock().await;
        if let Some(entry) = cache.get_mut(&worktree.id) {
            entry.touch();
            if now.duration_since(entry.value.cached_at) <= CACHE_TTL {
                entry.value.files.clone()
            } else {
                drop(cache);
                load_and_cache_worktree_files(&state, &worktree, now).await?
            }
        } else {
            drop(cache);
            load_and_cache_worktree_files(&state, &worktree, now).await?
        }
    };

    Ok(Json(completions::filter_and_rank_paths(
        &files, &query, limit,
    )))
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, String> {
    match raw {
        Some(value) => {
            let normalized = value.trim();
            if normalized.eq_ignore_ascii_case("true") || normalized == "1" {
                Ok(true)
            } else if normalized.is_empty()
                || normalized.eq_ignore_ascii_case("false")
                || normalized == "0"
            {
                Ok(false)
            } else {
                Err(format!("{label} must be true/false or 1/0"))
            }
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::title_generation_local;
    use std::collections::HashMap;

    use ctx_providers::fake::FakeProviderAdapter;
    use ctx_store::StoreManager;

    async fn setup_state() -> (tempfile::TempDir, Arc<AppState>, Session) {
        let data_dir = tempfile::tempdir().unwrap();
        let stores = StoreManager::open(data_dir.path()).await.unwrap();

        let workspace = stores
            .global()
            .create_workspace(
                "ws".to_string(),
                data_dir.path().to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .unwrap();
        let store = stores.workspace(workspace.id).await.unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                data_dir.path().to_string_lossy().to_string(),
                "base".to_string(),
                None,
            )
            .await
            .unwrap();
        stores
            .global()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
            .unwrap();
        let task = store
            .create_task(
                workspace.id,
                title_generation::DEFAULT_SESSION_TITLE.to_string(),
                None,
            )
            .await
            .unwrap();
        stores
            .global()
            .upsert_workspace_task_index(task.id, workspace.id)
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                "fake".to_string(),
                "fake-model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        stores
            .global()
            .upsert_workspace_session_index(session.id, workspace.id)
            .await
            .unwrap();

        let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
            HashMap::new();
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            stores,
            providers,
            "http://127.0.0.1:0".to_string(),
            None,
        ));

        (data_dir, state, session)
    }

    #[tokio::test]
    async fn schedule_title_generation_falls_back_without_config() {
        let (_data_dir, state, session) = setup_state().await;
        let prompt = "make the title this: hello world";
        let spawned = schedule_session_title_generation(
            state.clone(),
            session.clone(),
            prompt.to_string(),
            false,
        )
        .await;

        assert!(!spawned);

        let store = state.store_for_session(session.id).await.unwrap();
        let updated = store.get_session(session.id).await.unwrap().unwrap();
        let expected = title_generation::fallback_title_from_prompt(prompt);
        assert_eq!(updated.title, expected);
    }

    #[tokio::test]
    async fn generate_title_falls_back_when_local_runtime_missing() {
        let data_dir = tempfile::tempdir().unwrap();
        let model_path = title_generation_local::model_path(data_dir.path());
        if let Some(parent) = model_path.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&model_path, b"stub").await.unwrap();

        let cfg = user_settings::TitleGenerationSettings {
            mode: user_settings::TitleGenerationMode::Local,
            local: user_settings::TitleGenerationLocalSettings {
                model_id: title_generation_local::LOCAL_MODEL_ID.to_string(),
                use_json: true,
            },
            ..Default::default()
        };

        let prompt = "make the title this: hello world";
        let outcome = generate_title_for_prompt(Some(&cfg), prompt, data_dir.path())
            .await
            .unwrap();

        assert!(matches!(outcome.source, TitleGenerationSource::Fallback));
        assert_eq!(
            outcome.title,
            title_generation::fallback_title_from_prompt(prompt)
        );
    }

    fn result_with_status(status: &str) -> AgentInitResult {
        AgentInitResult {
            label: "agent".to_string(),
            status: status.to_string(),
            content: None,
            context_window: None,
            worktree_path: None,
        }
    }

    #[test]
    fn aggregate_subagent_status_reports_unknown() {
        let results = vec![
            result_with_status("completed"),
            result_with_status("unknown"),
        ];
        assert_eq!(aggregate_subagent_status(&results), "unknown");
    }

    #[test]
    fn aggregate_subagent_status_prefers_running_over_unknown() {
        let results = vec![result_with_status("running"), result_with_status("unknown")];
        assert_eq!(aggregate_subagent_status(&results), "running");
    }
}

pub(super) async fn delete_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let msg_id = MessageId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let workspaces = state
        .global_store()
        .list_workspaces()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut found: Option<(ctx_store::Store, Message)> = None;
    for workspace in workspaces {
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(msg) = store
            .get_message(msg_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            found = Some((store, msg));
            break;
        }
    }
    let Some((store, msg)) = found else {
        return Err(StatusCode::NOT_FOUND);
    };

    if !matches!(msg.delivery, MessageDelivery::Queued) || msg.delivered_at.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    store
        .delete_message(msg_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(turn_id) = msg.turn_id {
        let _ = store.delete_session_turn(msg.session_id, turn_id).await;
    }
    let removed = store
        .append_session_event(
            msg.session_id,
            msg.run_id,
            msg.turn_id,
            SessionEventType::MessageQueueRemoved,
            serde_json::json!({
                "message_id": msg.id.0,
                "reason": "user_delete",
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(removed).await;

    if let Some(tx) = state.scheduler_sender(msg.session_id).await {
        let _ = tx.send(SchedulerCommand::RemoveQueued(msg_id)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(super) struct PostMessageReq {
    content: String,
    delivery: Option<MessageDelivery>,
    #[serde(default)]
    attachments: Vec<MessageAttachment>,
}

async fn normalize_message_attachments(
    state: &Arc<AppState>,
    attachments: Vec<MessageAttachment>,
) -> Result<Vec<MessageAttachment>, StatusCode> {
    let mut out = Vec::with_capacity(attachments.len());
    for att in attachments {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                let saved =
                    persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref()).await?;
                out.push(MessageAttachment::ImageRef {
                    blob_id: saved.blob_id,
                    mime_type,
                    name,
                });
            }
            MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                name,
            } => {
                let exists = state
                    .global_store()
                    .get_blob(&blob_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .is_some();
                if !exists {
                    return Err(StatusCode::BAD_REQUEST);
                }
                out.push(MessageAttachment::ImageRef {
                    blob_id,
                    mime_type,
                    name,
                });
            }
        }
    }
    Ok(out)
}

impl TitleGenerationSource {}

pub(super) async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    state.remember_session_meta(&session).await;

    let delivery = match req.delivery {
        Some(d) => d,
        None => {
            if state.is_running(session_id).await {
                MessageDelivery::Queued
            } else {
                MessageDelivery::Immediate
            }
        }
    };

    let attachments = normalize_message_attachments(&state, req.attachments).await?;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let msg = Message {
        id: MessageId::new(),
        session_id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        role: MessageRole::User,
        content: req.content,
        attachments,
        delivery,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = store
        .insert_message(msg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let event = store
        .append_session_event(
            session_id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let start_seq = event.seq;

    let turn_status = if matches!(saved.delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: turn_status,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    if matches!(saved.delivery, MessageDelivery::Queued) {
        let queued = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::InputQueued,
                serde_json::json!({"message_id": saved.id.0}),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.publish_event(queued).await;

        let queue_position = store
            .list_queued_messages_for_session(session_id)
            .await
            .ok()
            .and_then(|messages| {
                messages
                    .iter()
                    .position(|message| message.id == saved.id)
                    .map(|idx| idx as i64)
            });

        let queue_added = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::MessageQueueAdded,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.publish_event(queue_added).await;

        let turn_queued = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnQueued,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.publish_event(turn_queued).await;
    }

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: run_id_header.clone(),
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    if let Ok(count) = store.count_user_messages_for_session(session_id).await {
        if count == 1 {
            let prompt = saved.content.clone();
            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }

    Ok(Json(saved))
}

pub(super) async fn list_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let subs = store
        .list_subagent_sessions(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(subs))
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct SessionSubagentInvocationsQuery {
    turn_id: Option<String>,
}

pub(super) async fn list_session_subagent_invocations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSubagentInvocationsQuery>,
) -> Result<Json<Vec<SubagentInvocation>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let turn_id = match q.turn_id {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(TurnId(
                    uuid::Uuid::parse_str(trimmed).map_err(|_| StatusCode::BAD_REQUEST)?,
                ))
            }
        }
        None => None,
    };

    let invocations = store
        .list_subagent_invocations_for_session(session.id, turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(invocations))
}

pub(super) async fn get_subagent_invocation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let workspaces = state
        .global_store()
        .list_workspaces()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for workspace in workspaces {
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(invocation) = store
            .get_subagent_invocation(&id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            return Ok(Json(invocation));
        }
    }
    Err(StatusCode::NOT_FOUND)
}

pub(super) async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Cancel).await;
    Ok(StatusCode::OK)
}

pub(super) async fn interrupt_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Interrupt).await;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub(super) struct SetSessionModelReq {
    model_id: String,
}

pub(super) async fn set_session_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_model(session.id.0.to_string(), req.model_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    store
        .update_session_model(session_id, req.model_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let worktree = store.get_worktree(updated.worktree_id).await.ok().flatten();
    Ok(Json(SessionWithEnv {
        env_target: env_target_for_worktree(worktree.as_ref()),
        session: updated,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct SetSessionModeReq {
    mode_id: String,
}

pub(super) async fn set_session_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModeReq>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_mode(session.id.0.to_string(), req.mode_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({"set_mode": req.mode_id}),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(event).await;

    Ok(StatusCode::OK)
}

const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;

fn resolve_max_subagents_per_call(settings: &user_settings::Settings) -> usize {
    let configured = settings
        .subagents
        .as_ref()
        .and_then(|s| s.max_per_call)
        .filter(|value| *value > 0);
    configured
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}
const DEFAULT_REASONING_EFFORT: &str = "medium";
const KNOWN_EFFORT_IDS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Deserialize)]
pub(super) struct AgentInitReq {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
    #[serde(default)]
    worktree: Option<String>,
    agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct AgentInitItem {
    prompt: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubagentWorktreeSelection {
    Inherit,
    New,
}

fn parse_subagent_worktree(value: Option<&str>) -> Result<SubagentWorktreeSelection, String> {
    let trimmed = value.map(|raw| raw.trim()).filter(|raw| !raw.is_empty());
    match trimmed {
        Some("inherit") => Ok(SubagentWorktreeSelection::Inherit),
        Some("new") => Ok(SubagentWorktreeSelection::New),
        Some(_) => Err("worktree must be 'inherit' or 'new'".to_string()),
        None => Err("worktree is required".to_string()),
    }
}

fn build_subagent_request_json(agents: &[AgentInitItem]) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        let prompt_length = prompt.chars().count() as u64;
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt_length)),
        );
        if let Some(label) = agent
            .label
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
        if let Some(harness) = agent
            .harness
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "harness".to_string(),
                serde_json::Value::String(harness.to_string()),
            );
        }
        if let Some(model) = agent
            .model
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) = agent
            .reasoning_effort
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let norm = normalize_effort_id(reasoning_effort);
            if !norm.is_empty() {
                obj.insert(
                    "reasoning_effort".to_string(),
                    serde_json::Value::String(norm),
                );
            }
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

#[derive(Debug, Serialize)]
pub(super) struct AgentInitResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(super) struct AgentInitResult {
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ContextWindowSummary {
    total: u64,
    used: u64,
    remaining: u64,
    utilization: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct SubagentWaitReq {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct SubagentWaitResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SubagentInterruptReq {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct SubagentInterruptResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(super) struct SubagentListItem {
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentReplyReq {
    label: String,
    prompt: String,
}

#[derive(Debug, Serialize)]
pub(super) struct AgentReplyResp {
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelInfo {
    base: String,
    effort: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelCatalog {
    full_ids: Vec<String>,
    base_ids: Vec<String>,
    efforts_by_base: HashMap<String, Vec<String>>,
    full_id_by_base_effort: HashMap<String, HashMap<String, String>>,
    info_by_full_id: HashMap<String, ModelInfo>,
}

#[derive(Debug, Clone)]
struct ResolvedModel {
    model_id: String,
}

fn normalize_effort_id(value: &str) -> String {
    let raw = value.trim().to_lowercase();
    match raw.as_str() {
        "extra_high" | "extra-high" | "extra high" | "extrahigh" => "xhigh".to_string(),
        _ => raw,
    }
}

fn is_known_effort_id(value: &str) -> bool {
    let norm = normalize_effort_id(value);
    KNOWN_EFFORT_IDS.iter().any(|id| *id == norm)
}

fn split_model_id(full: &str) -> (String, Option<String>) {
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if let Some(idx) = trimmed.rfind('/') {
        if idx > 0 && idx + 1 < trimmed.len() {
            let base = trimmed[..idx].to_string();
            let suffix = trimmed[idx + 1..].trim().to_string();
            if !suffix.is_empty() {
                return (base, Some(suffix));
            }
        }
    }
    (trimmed.to_string(), None)
}

fn has_trailing_paren_suffix(name: &str, suffix: &str) -> bool {
    let trimmed = name.trim_end();
    if !trimmed.ends_with(')') {
        return false;
    }
    let Some(start) = trimmed.rfind('(') else {
        return false;
    };
    let inner = trimmed[start + 1..trimmed.len() - 1].trim();
    normalize_effort_id(inner) == normalize_effort_id(suffix)
}

fn order_effort_ids(list: &mut [String]) {
    let order_index = |value: &str| {
        let norm = normalize_effort_id(value);
        KNOWN_EFFORT_IDS
            .iter()
            .position(|id| *id == norm)
            .unwrap_or(usize::MAX)
    };
    list.sort_by(|a, b| {
        let ia = order_index(a);
        let ib = order_index(b);
        if ia != ib {
            return ia.cmp(&ib);
        }
        a.cmp(b)
    });
}

fn build_model_catalog(models: &serde_json::Value) -> Option<ModelCatalog> {
    let entries = extract_model_entries(models);
    if entries.is_empty() {
        return None;
    }
    let mut full_ids = HashSet::new();
    let mut base_ids = HashSet::new();
    let mut info_by_full_id = HashMap::new();
    let mut raw_efforts_by_base: HashMap<String, HashSet<String>> = HashMap::new();
    let mut full_id_by_base_effort: HashMap<String, HashMap<String, String>> = HashMap::new();

    for (id, name) in entries {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (base_candidate, suffix) = split_model_id(trimmed);
        let effort = suffix.and_then(|s| {
            if is_known_effort_id(&s)
                || name
                    .as_deref()
                    .map(|n| has_trailing_paren_suffix(n, &s))
                    .unwrap_or(false)
            {
                Some(s)
            } else {
                None
            }
        });
        let base = if effort.is_some() {
            base_candidate
        } else {
            trimmed.to_string()
        };
        base_ids.insert(base.clone());
        full_ids.insert(trimmed.to_string());
        info_by_full_id.insert(
            trimmed.to_string(),
            ModelInfo {
                base: base.clone(),
                effort: effort.clone(),
            },
        );
        if let Some(effort) = effort {
            raw_efforts_by_base
                .entry(base.clone())
                .or_default()
                .insert(effort.clone());
            full_id_by_base_effort
                .entry(base)
                .or_default()
                .insert(normalize_effort_id(&effort), trimmed.to_string());
        }
    }

    let mut efforts_by_base = HashMap::new();
    for (base, efforts) in raw_efforts_by_base {
        let mut list = efforts.into_iter().collect::<Vec<_>>();
        order_effort_ids(&mut list);
        efforts_by_base.insert(base, list);
    }

    let mut full_ids = full_ids.into_iter().collect::<Vec<_>>();
    full_ids.sort();
    let mut base_ids = base_ids.into_iter().collect::<Vec<_>>();
    base_ids.sort();

    Some(ModelCatalog {
        full_ids,
        base_ids,
        efforts_by_base,
        full_id_by_base_effort,
        info_by_full_id,
    })
}

fn pick_default_effort(efforts: &[String]) -> Option<String> {
    let medium = efforts
        .iter()
        .find(|e| normalize_effort_id(e) == DEFAULT_REASONING_EFFORT)
        .cloned();
    medium.or_else(|| efforts.first().cloned())
}

fn resolve_model_id(
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fallback_model: Option<&str>,
    catalog: Option<&ModelCatalog>,
) -> Result<ResolvedModel, String> {
    let mut model = requested_model
        .or(fallback_model)
        .unwrap_or("")
        .trim()
        .to_string();
    if model.is_empty() {
        return Err("model is required".to_string());
    }
    let effort_input = requested_effort
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| e.to_string());

    if let Some(catalog) = catalog {
        let model_known = catalog.full_ids.contains(&model) || catalog.base_ids.contains(&model);
        if !model_known && requested_model.is_some() {
            return Err(format!(
                "unknown model '{model}'; available models: {}",
                catalog.full_ids.join(", ")
            ));
        }

        let info = catalog.info_by_full_id.get(&model);
        let base = info
            .map(|i| i.base.clone())
            .unwrap_or_else(|| model.clone());
        let existing_effort = info.and_then(|i| i.effort.clone());
        let available_efforts = catalog
            .efforts_by_base
            .get(&base)
            .cloned()
            .unwrap_or_default();
        let supports_default_effort = available_efforts.len() >= 2;
        let effort_map = catalog.full_id_by_base_effort.get(&base);

        if let Some(req_effort) = effort_input {
            let req_norm = normalize_effort_id(&req_effort);
            if let Some(existing) = existing_effort.as_ref() {
                if normalize_effort_id(existing) != req_norm {
                    return Err(format!(
                        "model '{model}' already includes effort '{existing}'; requested '{req_effort}'"
                    ));
                }
                return Ok(ResolvedModel { model_id: model });
            }
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&req_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
            let efforts = if available_efforts.is_empty() {
                effort_map
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                available_efforts.clone()
            };
            if efforts.is_empty() {
                return Err(format!("model '{base}' does not support reasoning_effort"));
            }
            return Err(format!(
                "invalid reasoning_effort '{req_effort}' for model '{base}'; available: {}",
                efforts.join(", ")
            ));
        }

        if existing_effort.is_some() {
            return Ok(ResolvedModel { model_id: model });
        }

        if supports_default_effort {
            if let Some(default_effort) = pick_default_effort(&available_efforts) {
                let default_norm = normalize_effort_id(&default_effort);
                if let Some(map) = effort_map {
                    if let Some(full_id) = map.get(&default_norm) {
                        return Ok(ResolvedModel {
                            model_id: full_id.clone(),
                        });
                    }
                }
            }
        } else if available_efforts.len() == 1 {
            let default_effort = available_efforts[0].clone();
            let default_norm = normalize_effort_id(&default_effort);
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&default_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
        }

        return Ok(ResolvedModel { model_id: model });
    }

    if let Some(req_effort) = effort_input {
        let (_, suffix) = split_model_id(&model);
        if suffix.is_none() {
            model = format!("{}/{}", model, req_effort);
            return Ok(ResolvedModel { model_id: model });
        }
    }

    Ok(ResolvedModel { model_id: model })
}

async fn load_provider_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<Option<ModelCatalog>, String> {
    let cache_key = format!("{}/{}", workspace.id.0, provider_id);
    if let Some(entry) = state.provider_options_cache.lock().await.get(&cache_key) {
        if let Some(models) = entry.value.get("models") {
            if let Some(catalog) = build_model_catalog(models) {
                return Ok(Some(catalog));
            }
        }
    }

    let supports_acp = {
        let statuses = state.provider_statuses.lock().await;
        statuses
            .get(provider_id)
            .and_then(|status| status.capabilities.as_ref())
            .map(|caps| caps.supports_acp)
            .unwrap_or(true)
    };
    if !supports_acp && provider_id != "codex-crp" {
        return Ok(None);
    }

    let cfg = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    let (command, args) = cfg
        .providers
        .get(provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&matrix, &state.data_root, provider_id))
        .ok_or_else(|| "unknown provider id".to_string())?;

    if !supports_acp {
        let mut env = std::collections::HashMap::new();
        env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
        if let Some(token) = state.auth_token.as_ref() {
            env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
        }

        let probe = match probe_crp_models(
            provider_id,
            command,
            args,
            PathBuf::from(&workspace.root_path),
            env,
        )
        .await
        {
            Ok(probe) => probe,
            Err(e) => {
                tracing::warn!(
                    provider_id = provider_id,
                    "provider options probe failed: {}",
                    logs::redact_sensitive(&e.to_string())
                );
                return Ok(None);
            }
        };

        let models_value = serde_json::json!({
            "models": probe.models,
            "current_model_id": probe.current_model_id,
        });
        if let Some(models) = build_model_catalog(&models_value) {
            let mut value = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": workspace.id.0,
                "installed": true,
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "models": models_value,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            value = redact_json_value(value);
            state.provider_options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
            return Ok(Some(models));
        }

        return Ok(None);
    }

    let agent = AcpAgentConfig {
        provider_id: provider_id.to_string(),
        command,
        args,
    };
    let client = AcpClientConfig {
        client_name: "ctx".to_string(),
        client_title: "ctx".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        system_prompt_append: None,
        mcp_servers: vec![],
    };
    let mut env = std::collections::HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }

    let probe =
        match probe_provider_options(agent, client, PathBuf::from(&workspace.root_path), env).await
        {
            Ok(probe) => probe,
            Err(e) => {
                tracing::warn!(
                    provider_id = provider_id,
                    "provider options probe failed: {}",
                    logs::redact_sensitive(&e.to_string())
                );
                return Ok(None);
            }
        };

    if let Some(models) = probe.models.as_ref().and_then(build_model_catalog) {
        let mut value = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace.id.0,
            "installed": true,
            "probe_ok": true,
            "supports_load": probe.supports_load,
            "auth_required": probe.auth_required,
            "auth_methods": probe.auth_methods,
            "modes": probe.modes,
            "models": probe.models,
            "acp_error": probe.acp_error,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
        value = redact_json_value(value);
        state.provider_options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value,
            },
        );
        return Ok(Some(models));
    }

    Ok(None)
}

async fn wait_for_run_terminal_event(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<SessionEventType, String> {
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    if let Some(event) = store
        .get_terminal_event_for_run(session_id, run_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    {
        return Ok(event.event_type);
    }

    let mut rx = state.get_broadcaster(session_id).await.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if event.run_id == Some(run_id)
                    && matches!(
                        event.event_type,
                        SessionEventType::Done
                            | SessionEventType::Error
                            | SessionEventType::TurnInterrupted
                            | SessionEventType::TurnFinished
                    )
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Some(event) = store
                    .get_terminal_event_for_run(session_id, run_id)
                    .await
                    .map_err(|e| logs::redact_sensitive(&e.to_string()))?
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err("session event stream closed".to_string());
            }
        }
    }
}

async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let event = store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;
    Ok(())
}

fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(num) => num.as_u64(),
        serde_json::Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(raw) => raw.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn summarize_context_window(metrics: &serde_json::Value) -> Option<ContextWindowSummary> {
    let obj = metrics.as_object()?;
    let total = obj
        .get("context_window_tokens")
        .and_then(parse_u64)
        .or_else(|| obj.get("context_window").and_then(parse_u64))
        .or_else(|| obj.get("window_tokens").and_then(parse_u64))?;
    let mut used = obj
        .get("context_tokens_estimate")
        .and_then(parse_u64)
        .or_else(|| obj.get("total_tokens").and_then(parse_u64))
        .or_else(|| obj.get("used_tokens").and_then(parse_u64));
    let mut remaining = obj
        .get("remaining_tokens_estimate")
        .and_then(parse_u64)
        .or_else(|| obj.get("remaining_tokens").and_then(parse_u64));

    if used.is_none() {
        if let Some(rem) = remaining {
            used = Some(total.saturating_sub(rem));
        }
    }
    if remaining.is_none() {
        if let Some(used) = used {
            remaining = Some(total.saturating_sub(used));
        }
    }
    let used = used.unwrap_or(0);
    let remaining = remaining.unwrap_or_else(|| total.saturating_sub(used));
    let utilization = obj
        .get("remaining_fraction")
        .and_then(parse_f64)
        .map(|fraction| (1.0 - fraction).clamp(0.0, 1.0))
        .unwrap_or_else(|| {
            if total == 0 {
                0.0
            } else {
                (used as f64 / total as f64).clamp(0.0, 1.0)
            }
        });

    Some(ContextWindowSummary {
        total,
        used,
        remaining,
        utilization,
    })
}

async fn context_window_for_run(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Option<ContextWindowSummary> {
    let store = state.store_for_session(session_id).await.ok()?;
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .ok()
        .flatten()?;
    summarize_context_window(turn.metrics_json.as_ref()?)
}

async fn context_window_for_session(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Option<ContextWindowSummary> {
    let store = state.store_for_session(session_id).await.ok()?;
    let turn = store
        .get_latest_turn_for_session(session_id)
        .await
        .ok()
        .flatten()?;
    summarize_context_window(turn.metrics_json.as_ref()?)
}

fn estimate_context_window_for_prompt_len(
    provider_id: &str,
    model_id: &str,
    prompt_len: i64,
) -> Option<ContextWindowSummary> {
    let total = crate::scheduler::model_context_window(provider_id, model_id)? as u64;
    let chars = prompt_len.max(0) as u64;
    let used = chars.div_ceil(4);
    let remaining = total.saturating_sub(used);
    let utilization = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };
    Some(ContextWindowSummary {
        total,
        used,
        remaining,
        utilization,
    })
}

fn estimate_context_window_for_prompt(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<ContextWindowSummary> {
    let prompt_len = prompt.chars().count() as i64;
    estimate_context_window_for_prompt_len(provider_id, model_id, prompt_len)
}

async fn worktree_path_for_child(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    child_session_id: SessionId,
) -> Option<String> {
    let store = state.store_for_session(child_session_id).await.ok()?;
    let session = store.get_session(child_session_id).await.ok().flatten()?;
    if session.worktree_id == parent_worktree_id {
        return None;
    }
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten()?;
    Some(worktree.root_path)
}

async fn build_subagent_result(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    child: &SubagentInvocationChild,
    status: String,
    content: Option<String>,
    context_window: Option<ContextWindowSummary>,
) -> Result<AgentInitResult, String> {
    let label = child
        .label
        .clone()
        .unwrap_or_else(|| format!("Subagent {}", child.position + 1));
    let worktree_path =
        worktree_path_for_child(state, parent_worktree_id, child.child_session_id).await;
    Ok(AgentInitResult {
        label,
        status,
        content,
        context_window,
        worktree_path,
    })
}

async fn build_subagent_result_for_session(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    session: &Session,
    label: String,
    status: String,
    content: Option<String>,
    context_window: Option<ContextWindowSummary>,
) -> Result<AgentInitResult, String> {
    let worktree_path = if session.worktree_id == parent_worktree_id {
        None
    } else {
        let store = state
            .store_for_session(session.id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
        store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?
            .map(|worktree| worktree.root_path)
    };
    Ok(AgentInitResult {
        label,
        status,
        content,
        context_window,
        worktree_path,
    })
}

async fn run_subagent_child(
    state: &Arc<AppState>,
    child: SubagentInvocationChild,
    parent_worktree_id: WorktreeId,
) -> Result<AgentInitResult, String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let terminal = wait_for_run_terminal_event(state, child.child_session_id, run_id).await;
    let status = match terminal {
        Ok(SessionEventType::Done) | Ok(SessionEventType::TurnFinished) => "completed",
        Ok(SessionEventType::TurnInterrupted) => "interrupted",
        Ok(SessionEventType::Error) => "failed",
        Ok(_) => "completed",
        Err(_) => "unknown",
    }
    .to_string();

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    store
        .upsert_subagent_invocation_child(updated_child)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;

    let content = store
        .get_last_assistant_message_for_run(child.child_session_id, run_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.content);

    let context_window = context_window_for_run(state, child.child_session_id, run_id).await;
    build_subagent_result(
        state,
        parent_worktree_id,
        &child,
        status,
        content,
        context_window,
    )
    .await
}

async fn finalize_subagent_invocation(
    state: &Arc<AppState>,
    invocation_id: &str,
    tool_call_id: &str,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
) -> Result<(), String> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let Some(invocation) = store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    else {
        return Ok(());
    };

    if invocation.children.is_empty() {
        return Ok(());
    }
    if invocation
        .children
        .iter()
        .any(|child| child.status == "running")
    {
        return Ok(());
    }

    let final_status = if invocation
        .children
        .iter()
        .all(|child| child.status == "completed")
    {
        "completed"
    } else {
        "failed"
    };
    if invocation.status == final_status {
        return Ok(());
    }

    let updated_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(invocation_id, final_status, updated_at)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let child_session_ids = invocation
        .children
        .iter()
        .map(|child| child.child_session_id.0.to_string())
        .collect::<Vec<_>>();
    let child_statuses = invocation
        .children
        .iter()
        .map(|child| {
            serde_json::json!({
                "session_id": child.child_session_id.0.to_string(),
                "status": child.status,
            })
        })
        .collect::<Vec<_>>();
    emit_subagent_invocation_notice(
        state,
        parent_session_id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": final_status,
            "child_session_ids": child_session_ids,
            "child_statuses": child_statuses,
        }),
    )
    .await
    .map_err(|(_, err)| err.0.error)?;

    Ok(())
}

async fn create_subagent_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    task_id: TaskId,
    base_commit_sha: &str,
    vcs_kind: VcsKind,
) -> Result<Worktree, (StatusCode, Json<ApiErrorResp>)> {
    let worktree_id = WorktreeId::new();
    let wt_path = managed_worktree_path(&state.data_root, workspace.id, worktree_id);
    if let Some(parent) = wt_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    }
    let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
    create_worktree(
        &workspace.root_path,
        &wt_path,
        base_commit_sha,
        &branch_name,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.to_string(),
        git_branch: (vcs_kind == VcsKind::Git).then(|| branch_name.clone()),
        vcs_kind: Some(vcs_kind),
        base_revision: Some(base_commit_sha.to_string()),
        vcs_ref: Some(branch_name),
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_config_path: None,
        bootstrap_config_key: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };

    store.insert_worktree(worktree.clone()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if let Err(e) = state
        .global_store()
        .upsert_workspace_worktree_index(worktree_id, workspace.id)
        .await
    {
        tracing::warn!(
            worktree_id = %worktree_id.0,
            "failed to update worktree index: {e:?}"
        );
    }
    if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(state),
        workspace.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "worktree bootstrap failed: {e:?}");
    }
    if let Err(e) =
        attachments::sync_workspace_attachments(Arc::clone(state), workspace, false).await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "attachment sync failed: {e:?}");
    }
    if let Err(e) =
        attachments::ensure_worktree_attachment_mounts_if_materialized(state, workspace, &worktree)
            .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "attachment mounts failed: {e:?}");
    }

    Ok(worktree)
}

async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> Result<(RunId, Message), (StatusCode, Json<ApiErrorResp>)> {
    let store = state.store_for_session(session.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let msg = Message {
        id: MessageId::new(),
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        role: MessageRole::User,
        content: prompt,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = store.insert_message(msg).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let event = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
            }),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let start_seq = event.seq;

    let turn = SessionTurn {
        turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: SessionTurnStatus::Running,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    Ok((run_id, saved))
}

pub(super) async fn mcp_agent_init(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentInitReq>,
) -> Result<Json<AgentInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if req.agents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "agents is required".to_string(),
            }),
        ));
    }
    let settings = user_settings::load_settings(&state.data_root).await;
    let max_subagents = resolve_max_subagents_per_call(&settings);
    if req.agents.len() > max_subagents {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("max {max_subagents} subagents per call"),
            }),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "response_mode is not supported; use subagent_wait to await".to_string(),
            }),
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let mut labels = Vec::with_capacity(req.agents.len());
    let mut seen_labels = HashSet::new();
    for (idx, agent) in req.agents.iter().enumerate() {
        let label = agent
            .label
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("agent {} label is required", idx + 1),
                }),
            ))?;
        if !seen_labels.insert(label.to_string()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate subagent label '{label}'"),
                }),
            ));
        }
        labels.push(label.to_string());
    }
    for label in &labels {
        if store
            .subagent_label_exists(parent.task_id, label)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent label '{label}' already exists for this task"),
                }),
            ));
        }
    }

    let mut provider_ids = HashSet::new();
    for agent in &req.agents {
        let provider_id = agent
            .harness
            .as_deref()
            .unwrap_or(&parent.provider_id)
            .trim();
        if provider_id.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "harness is required".to_string(),
                }),
            ));
        }
        provider_ids.insert(provider_id.to_string());
    }

    let available_providers: Vec<String> = {
        let statuses = state.provider_statuses.lock().await;
        let mut ids = statuses.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    };
    let mut provider_statuses = HashMap::new();
    {
        let statuses = state.provider_statuses.lock().await;
        for provider_id in provider_ids.iter() {
            if let Some(status) = statuses.get(provider_id) {
                provider_statuses.insert(provider_id.clone(), status.clone());
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!(
                            "unknown harness '{provider_id}'; available harnesses: {}",
                            available_providers.join(", ")
                        ),
                    }),
                ));
            }
        }
    }

    for (provider_id, status) in &provider_statuses {
        if !status.installed
            || !matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("harness '{provider_id}' is not installed or unhealthy"),
                }),
            ));
        }
    }

    let mut model_catalogs: HashMap<String, Option<ModelCatalog>> = HashMap::new();
    for provider_id in provider_ids.iter() {
        let catalog = load_provider_model_catalog(&state, &workspace, provider_id).await;
        match catalog {
            Ok(cat) => {
                model_catalogs.insert(provider_id.clone(), cat);
            }
            Err(err) => {
                return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: err })));
            }
        }
    }

    let parent_worktree = store
        .get_worktree(parent.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent worktree not found".to_string(),
            }),
        ))?;

    let worktree_plan = if worktree_selection == SubagentWorktreeSelection::New {
        let parent_root = StdPath::new(&parent_worktree.root_path);
        let vcs = vcs::driver_for_path(parent_root).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        let base_commit_sha = vcs.rev_parse_head(parent_root).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "git repo has no commits; create an initial commit before creating a worktree".to_string(),
                    }),
                );
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

        let (dirty_files, dirty_additions, dirty_deletions) =
            ctx_fs::worktrees::diff_worktree_summary(parent_root, &base_commit_sha)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        if dirty_files > 0 || dirty_additions > 0 || dirty_deletions > 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs.".to_string(),
                }),
            ));
        }
        Some((vcs.kind(), base_commit_sha))
    } else {
        None
    };

    let request_json = Some(build_subagent_request_json(&req.agents));

    let mut requested_tool_call_id = req
        .tool_call_id
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let invocation_id = requested_tool_call_id
        .clone()
        .unwrap_or_else(|| format!("subagent-{}", uuid::Uuid::new_v4()));
    let tool_call_id = requested_tool_call_id
        .take()
        .unwrap_or_else(|| invocation_id.clone());

    let mut parent_turn_id = None;
    if !tool_call_id.trim().is_empty() {
        if let Ok(Some(tool)) = store.get_session_turn_tool(parent.id, &tool_call_id).await {
            parent_turn_id = Some(tool.turn_id);
        }
    }
    if parent_turn_id.is_none() {
        if let Ok(turns) = store
            .list_session_turns_page_by_seq(parent.id, None, Some(5))
            .await
        {
            for turn in turns.iter().rev() {
                if matches!(
                    turn.status,
                    SessionTurnStatus::Running | SessionTurnStatus::Queued
                ) {
                    parent_turn_id = Some(turn.turn_id);
                    break;
                }
            }
        }
    }

    let now = chrono::Utc::now();
    let invocation = SubagentInvocation {
        id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        parent_session_id: parent.id,
        parent_turn_id,
        requested_count: req.agents.len() as i64,
        request_json,
        status: "requested".to_string(),
        created_at: now,
        updated_at: now,
        children: Vec::new(),
    };
    store
        .upsert_subagent_invocation(invocation)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_created",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "requested",
            "requested_count": req.agents.len(),
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let running_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(&invocation_id, "running", running_at)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "running",
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let child_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    let mut futures = Vec::with_capacity(req.agents.len());
    for (idx, agent) in req.agents.into_iter().enumerate() {
        let state = state.clone();
        let parent = parent.clone();
        let workspace = workspace.clone();
        let model_catalogs = model_catalogs.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let child_ids = child_ids.clone();
        let parent_turn_id = parent_turn_id;
        let label = labels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("Subagent {}", idx + 1));
        let worktree_plan = worktree_plan.clone();
        futures.push(async move {
            let store = state.store_for_session(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("agent {} prompt is required", idx + 1),
                    }),
                ));
            }
            let provider_id = agent
                .harness
                .as_deref()
                .unwrap_or(&parent.provider_id)
                .trim()
                .to_string();
            let catalog = model_catalogs.get(&provider_id).and_then(|v| v.as_ref());
            let fallback_model = if agent.model.is_none() {
                if provider_id == parent.provider_id {
                    Some(parent.model_id.as_str())
                } else {
                    catalog.and_then(|c| c.full_ids.first().map(|s| s.as_str()))
                }
            } else {
                None
            };
            if agent.model.is_none() && fallback_model.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("model is required for harness '{provider_id}'"),
                    }),
                ));
            }
            let resolved = resolve_model_id(
                agent.model.as_deref(),
                agent.reasoning_effort.as_deref(),
                fallback_model,
                catalog,
            )
            .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

            let prompt_length = prompt.chars().count() as i64;
            let requested_effort = agent
                .reasoning_effort
                .as_deref()
                .map(normalize_effort_id)
                .filter(|value| !value.is_empty());
            let (_, model_effort) = split_model_id(&resolved.model_id);
            let reasoning_effort = requested_effort.or(model_effort);

            let worktree_id = match worktree_selection {
                SubagentWorktreeSelection::Inherit => parent.worktree_id,
                SubagentWorktreeSelection::New => {
                    let (vcs_kind, base_commit_sha) = worktree_plan.clone().ok_or((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "worktree plan missing".to_string(),
                        }),
                    ))?;
                    let worktree = create_subagent_worktree(
                        &state,
                        &store,
                        &workspace,
                        parent.task_id,
                        &base_commit_sha,
                        vcs_kind,
                    )
                    .await?;
                    worktree.id
                }
            };

            let session = store
                .create_session(
                    parent.task_id,
                    parent.workspace_id,
                    worktree_id,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
            if let Err(e) = state
                .global_store()
                .upsert_workspace_session_index(session.id, parent.workspace_id)
                .await
            {
                tracing::warn!(
                    session_id = %session.id.0,
                    "failed to update subagent session index: {e:?}"
                );
            }

            if store
                .update_session_title(session.id, label.clone())
                .await
                .is_err()
            {
                tracing::warn!(session_id = %session.id.0, "failed to set subagent label");
            }

            let child_created_at = chrono::Utc::now();
            let (run_id, _message) = enqueue_subagent_prompt(&state, &session, prompt).await?;
            let child_session_id = session.id;
            let child = SubagentInvocationChild {
                invocation_id: invocation_id.clone(),
                child_session_id,
                run_id: Some(run_id),
                position: idx as i64,
                status: "running".to_string(),
                label: Some(label),
                harness: Some(provider_id),
                model: Some(resolved.model_id),
                reasoning_effort,
                prompt_length,
                created_at: child_created_at,
                updated_at: child_created_at,
            };
            store
                .upsert_subagent_invocation_child(child.clone())
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;

            let child_ids_snapshot = {
                let mut ids = child_ids.lock().await;
                let child_id_string = child_session_id.0.to_string();
                ids.push(child_id_string);
                ids.clone()
            };
            emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "running",
                    "child_session_ids": child_ids_snapshot,
                }),
            )
            .await?;

            Ok(child)
        });
    }

    let children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(err) => {
            let updated_at = chrono::Utc::now();
            if let Ok(store) = state.store_for_session(parent.id).await {
                if let Err(e) = store
                    .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                    .await
                {
                    tracing::warn!(error = ?e, "failed to update subagent invocation status");
                }
            }
            let child_session_ids = {
                let ids = child_ids.lock().await;
                ids.clone()
            };
            let _ = emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "failed",
                    "child_session_ids": child_session_ids,
                }),
            )
            .await;
            return Err(err);
        }
    };

    for child in children.iter().cloned() {
        let state = state.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let parent_id = parent.id;
        let parent_worktree_id = parent.worktree_id;
        tokio::spawn(async move {
            if let Err(error) = run_subagent_child(&state, child, parent_worktree_id).await {
                tracing::warn!(error = %error, "subagent execution failed");
            }
            if let Err(error) = finalize_subagent_invocation(
                &state,
                &invocation_id,
                &tool_call_id,
                parent_id,
                parent_turn_id,
            )
            .await
            {
                tracing::warn!(error = %error, "failed to finalize subagent invocation");
            }
        });
    }

    let results = futures::future::try_join_all(children.iter().map(|child| {
        let context_window = match (child.harness.as_deref(), child.model.as_deref()) {
            (Some(provider_id), Some(model_id)) => {
                estimate_context_window_for_prompt_len(provider_id, model_id, child.prompt_length)
            }
            _ => None,
        };
        build_subagent_result(
            &state,
            parent.worktree_id,
            child,
            "running".to_string(),
            None,
            context_window,
        )
    }))
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        )
    })?;

    Ok(Json(AgentInitResp {
        status: "running".to_string(),
        results,
    }))
}

pub(super) async fn mcp_agent_reply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentReplyReq>,
) -> Result<Json<AgentReplyResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let label = req.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "label is required".to_string(),
            }),
        ));
    }
    let child = store
        .get_subagent_session_by_label(parent.id, label)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "subagent label not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    if store
        .get_running_turn_for_session(child.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "subagent_busy: The subagent is still running. Please await it with subagent_wait or interrupt it with subagent_interrupt.".to_string(),
            }),
        ));
    }

    let mut context_window =
        estimate_context_window_for_prompt(&child.provider_id, &child.model_id, &prompt);
    if context_window.is_none() {
        context_window = context_window_for_session(&state, child.id).await;
    }

    let (_run_id, _message) = enqueue_subagent_prompt(&state, &child, prompt).await?;
    let worktree_path = worktree_path_for_child(&state, parent.worktree_id, child.id).await;

    Ok(Json(AgentReplyResp {
        label: label.to_string(),
        status: "running".to_string(),
        context_window,
        worktree_path,
    }))
}

pub(super) async fn mcp_subagent_list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SubagentListItem>>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let mut results = Vec::with_capacity(subs.len());
    for sub in subs {
        let label = sub.title.trim().to_string();
        let status = match sub.status {
            SessionStatus::Active => "active",
            SessionStatus::Completed => "completed",
            SessionStatus::Failed => "failed",
            SessionStatus::Cancelled => "cancelled",
        }
        .to_string();
        let context_window = context_window_for_session(&state, sub.id).await;
        let worktree_path = worktree_path_for_child(&state, parent.worktree_id, sub.id).await;
        results.push(SubagentListItem {
            label,
            status,
            context_window,
            worktree_path,
        });
    }

    Ok(Json(results))
}

pub(super) async fn mcp_subagent_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentInterruptReq>,
) -> Result<Json<SubagentInterruptResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let use_all = req.all.unwrap_or(false);
    let labels = match (use_all, req.label) {
        (true, Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or all".to_string(),
                }),
            ));
        }
        (true, None) => {
            let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            subs.into_iter()
                .map(|sub| sub.title.trim().to_string())
                .collect::<Vec<_>>()
        }
        (false, Some(label)) => vec![label],
        (false, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or all is required".to_string(),
                }),
            ));
        }
    };

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        let child = store
            .get_subagent_session_by_label(parent.id, trimmed)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: format!("subagent label '{trimmed}' not found"),
                }),
            ))?;

        let tx = state.ensure_scheduler(child.clone()).await;
        let _ = tx.send(SchedulerCommand::Interrupt).await;

        let context_window = context_window_for_session(&state, child.id).await;
        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                trimmed.to_string(),
                "interrupt_requested".to_string(),
                None,
                context_window,
            )
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    Ok(Json(SubagentInterruptResp {
        status: "interrupt_requested".to_string(),
        results,
    }))
}

pub(super) async fn mcp_oracle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<oracle::OracleRequest>,
) -> Result<Json<oracle::OracleResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let workspace_id = state
        .global_store()
        .get_workspace_id_for_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(workspace_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    store
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
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    let settings = user_settings::load_settings(&state.data_root).await;
    let cfg = settings.oracle.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "oracle is not configured".to_string(),
        }),
    ))?;

    let resp = oracle::oracle_one_shot(cfg, req).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(resp))
}

pub(super) async fn mcp_subagent_wait(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentWaitReq>,
) -> Result<Json<SubagentWaitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let mut labels = match (req.label, req.labels) {
        (Some(label), None) => vec![label],
        (None, Some(labels)) => labels,
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or labels".to_string(),
                }),
            ));
        }
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or labels is required".to_string(),
                }),
            ));
        }
    };
    if labels.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "labels is required".to_string(),
            }),
        ));
    }
    let mut seen = HashSet::new();
    for label in labels.iter_mut() {
        let trimmed = label.trim().to_string();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        if !seen.insert(trimmed.clone()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate label '{trimmed}'"),
                }),
            ));
        }
        *label = trimmed;
    }

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let child = store
            .get_subagent_session_by_label(parent.id, &label)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: format!("subagent label '{label}' not found"),
                }),
            ))?;

        let running_turn = store
            .get_running_turn_for_session(child.id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

        let (status, run_id) = if let Some(turn) = running_turn {
            let run_id = turn.run_id.ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "subagent run_id missing; cannot wait".to_string(),
                }),
            ))?;
            let terminal = wait_for_run_terminal_event(&state, child.id, run_id).await;
            let status = match terminal {
                Ok(SessionEventType::Done) | Ok(SessionEventType::TurnFinished) => "completed",
                Ok(SessionEventType::TurnInterrupted) => "interrupted",
                Ok(SessionEventType::Error) => "failed",
                Ok(_) => "completed",
                Err(_) => "unknown",
            }
            .to_string();
            (status, Some(run_id))
        } else if let Some(turn) =
            store
                .get_latest_turn_for_session(child.id)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?
        {
            let status = match turn.status {
                SessionTurnStatus::Completed => "completed",
                SessionTurnStatus::Interrupted => "interrupted",
                SessionTurnStatus::Failed => "failed",
                SessionTurnStatus::Running | SessionTurnStatus::Queued => "running",
            }
            .to_string();
            (status, turn.run_id)
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent '{label}' has no runs to wait for"),
                }),
            ));
        };

        let content = match run_id {
            Some(run_id) => store
                .get_last_assistant_message_for_run(child.id, run_id)
                .await
                .ok()
                .flatten()
                .map(|m| m.content),
            None => None,
        };
        let context_window = match run_id {
            Some(run_id) => context_window_for_run(&state, child.id, run_id).await,
            None => context_window_for_session(&state, child.id).await,
        };

        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                label,
                status,
                content,
                context_window,
            )
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    let status = aggregate_subagent_status(&results);

    Ok(Json(SubagentWaitResp {
        status: status.to_string(),
        results,
    }))
}

fn aggregate_subagent_status(results: &[AgentInitResult]) -> &'static str {
    if results.iter().any(|r| r.status == "failed") {
        "failed"
    } else if results.iter().any(|r| r.status == "interrupted") {
        "interrupted"
    } else if results.iter().any(|r| r.status == "running") {
        "running"
    } else if results.iter().any(|r| r.status == "unknown") {
        "unknown"
    } else {
        "completed"
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct GenerateSessionTitleReq {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

pub(super) async fn generate_session_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<GenerateSessionTitleReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = if let Some(prompt) = req
        .prompt
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        prompt
    } else {
        store
            .get_first_user_message_content(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .filter(|p| !p.trim().is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "prompt required".to_string(),
                }),
            ))?
    };

    let force = req.force.unwrap_or(true);
    let cfg = configured_title_generation_settings(&state).await;
    maybe_generate_session_title(state.clone(), session.clone(), prompt, force, cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title generation skipped".to_string(),
            }),
        ))?;

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let updated = store
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
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthenticateSessionReq {
    #[serde(default)]
    method_id: Option<String>,
}

pub(super) async fn authenticate_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AuthenticateSessionReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(session_id).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
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

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "provider adapter not available".to_string(),
            }),
        )
    })?;

    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
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

    let workdir = PathBuf::from(worktree.root_path.clone());

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }
    if session.provider_id == "codex" {
        if let Ok(extra) = provider_accounts::codex_env_for_active_account(&state.data_root).await {
            for (key, value) in extra {
                provider_env.insert(key, value);
            }
        }
    }

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let state_for_events = state.clone();
    let store_for_events = store.clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let payload = ev.payload_json.clone();
            if matches!(ev.event_type, SessionEventType::Init) {
                if let Some(ps) = payload
                    .get("acp_session_id")
                    .and_then(serde_json::Value::as_str)
                {
                    let _ = store_for_events
                        .update_session_provider_session_ref(session_id, Some(ps.to_string()))
                        .await;
                }
            }
            let appended = store_for_events
                .append_session_event(session_id, None, None, ev.event_type.clone(), payload)
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event).await;
            }
        }
    });

    let started = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "auth_started",
                "provider": session.provider_id,
                "method_id": req.method_id,
            }),
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to append auth event".to_string(),
                }),
            )
        })?;
    state.publish_event(started).await;

    let session_key = session.id.0.to_string();
    let result = adapter
        .authenticate_session(
            session_key,
            workdir,
            provider_env,
            req.method_id.clone(),
            ev_tx,
        )
        .await;

    match result {
        Ok(()) => {
            let done = store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_finished",
                        "provider": session.provider_id,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(done).await;
            Ok(StatusCode::OK)
        }
        Err(e) => {
            let msg = logs::redact_sensitive(&e.to_string());
            let failed = store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_failed",
                        "provider": session.provider_id,
                        "message": msg,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(failed).await;
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "authentication failed".to_string(),
                }),
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct SubmitAskUserQuestionReq {
    tool_call_id: String,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    answers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct SubmitAskUserQuestionResp {
    ok: bool,
}

pub(super) async fn submit_ask_user_question(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubmitAskUserQuestionReq>,
) -> Result<Json<SubmitAskUserQuestionResp>, (StatusCode, Json<ApiErrorResp>)> {
    let session_uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?;
    let session_id = SessionId(session_uuid);

    // Validate the session exists (prevents accidentally fulfilling a prompt for a deleted session).
    let store = state.store_for_session(session_id).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let exists = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .is_some();
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }

    let tool_call_id = req.tool_call_id.trim().to_string();
    if tool_call_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing tool_call_id".to_string(),
            }),
        ));
    }

    let outcome = match req.outcome.as_deref() {
        Some("cancelled") => AskUserQuestionOutcome::Cancelled,
        Some("submitted") | None => AskUserQuestionOutcome::Submitted,
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("invalid outcome: {other}"),
                }),
            ))
        }
    };
    let answers = req.answers.unwrap_or_default();
    let answers_for_event = answers.clone();

    let ok = state
        .ask_user_question
        .submit(
            &session_uuid.to_string(),
            &tool_call_id,
            AskUserQuestionAnswer { outcome, answers },
        )
        .await;

    if !ok {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "no pending AskUserQuestion for this tool_call_id".to_string(),
            }),
        ));
    }

    if let Ok(store) = state.store_for_session(session_id).await {
        if let Ok(event) = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({
                    "kind": "ask_user_question_answered",
                    "tool_call_id": tool_call_id,
                    "outcome": outcome.as_str(),
                    "answers": answers_for_event,
                }),
            )
            .await
        {
            state.publish_event(event).await;
        }
    }

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
}
