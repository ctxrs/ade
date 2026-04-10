use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::Engine;
use sha2::Digest;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::artifacts::persist_blob_bytes;
use super::errors::ApiErrorResp;
use super::extractors::extract_model_entries;
use super::redact_json_value;
use super::shared::{load_and_cache_worktree_files, FileCompletionsQuery};
use crate::completions;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::git_status::{load_git_status_snapshot, GitStatusEntry};
use crate::installer;
use crate::logs;
use crate::oracle;
use crate::order_seq::attach_order_seq;
use crate::scheduler::SchedulerCommand;
use crate::settings as user_settings;
use ctx_harness_runtime::sandbox_container_command;
use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_fs::vcs;
use ctx_provider_accounts as provider_accounts;
use ctx_providers::events::NormalizedEvent;
use ctx_providers::{
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
    crp::probe_crp_models,
};
use ctx_sandbox_container_runtime::command_output_with_timeout;
use ctx_store::is_unique_constraint_violation;
use ctx_workspace_container::workspace_container_name;
use tokio::sync::mpsc;

mod subagents;
pub(super) use subagents::{
    get_session_subagent_invocation, list_session_subagent_invocations, list_session_subagents,
    mcp_agent_init, mcp_agent_reply, mcp_oracle, mcp_subagent_interrupt, mcp_subagent_list,
    mcp_subagent_wait,
};
mod diff_exec;
use diff_exec::{diff_worktree_for_session, diff_worktree_summary_for_session};
mod control;
pub(super) use control::{
    authenticate_session, cancel_session, interrupt_session, submit_ask_user_question,
};
mod file_completions;
pub(super) use file_completions::session_file_completions;
mod messages;
pub(crate) use messages::ensure_session_turn_for_message;
pub(super) use messages::{delete_session_message, post_message};
mod models;
pub(crate) use models::{
    compose_model_id, deserialize_optional_reasoning_effort,
    load_provider_model_catalog_for_execution_environment, normalize_effort_id, resolve_model_id,
};
mod snapshot;
pub(super) use snapshot::{
    apply_session_diff_patch, get_session_diff, get_session_diff_summary, get_session_events,
    get_session_git_status, get_session_head, get_session_history, get_session_snapshot,
    get_session_state, list_session_turn_tools,
};
pub(crate) use snapshot::{
    is_no_vcs_repo_error, resolve_diff_base_with_meta, SessionDiffQuery, WorktreeDiffBaseResolution,
};
mod titles_and_modes;
#[cfg(test)]
use crate::title_generation;
pub(super) use titles_and_modes::{
    generate_session_title, schedule_session_title_generation, set_session_mode, set_session_model,
};
#[cfg(test)]
use titles_and_modes::{generate_title_for_prompt, TitleGenerationSource};

async fn store_for_existing_session_status(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    match state.lookup_session_store(session_id).await {
        crate::daemon::StoreLookup::Found(store) => Ok(store),
        crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
            Err(StatusCode::NOT_FOUND)
        }
        crate::daemon::StoreLookup::Unavailable(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

const STORE_OPEN_RETRY_LIMIT: usize = 3;
const STORE_OPEN_RETRY_BASE_MS: u64 = 40;

fn is_transient_store_open_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

async fn store_for_existing_session_status_with_retry(
    state: &Arc<AppState>,
    session_id: SessionId,
    retry_limit: usize,
    retry_base_ms: u64,
) -> Result<ctx_store::Store, StatusCode> {
    let mut attempt = 0usize;
    loop {
        match state.lookup_session_store(session_id).await {
            crate::daemon::StoreLookup::Found(store) => return Ok(store),
            crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
                return Err(StatusCode::NOT_FOUND);
            }
            crate::daemon::StoreLookup::Unavailable(err) => {
                if is_transient_store_open_error(&err) && attempt < retry_limit {
                    attempt += 1;
                    let backoff_ms = retry_base_ms.saturating_mul(attempt as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                tracing::warn!(session_id = %session_id.0, "session store lookup failed: {err:#}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
}

async fn store_for_existing_session_status_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await
}

async fn store_for_existing_session_api_error(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    match state.lookup_session_store(session_id).await {
        crate::daemon::StoreLookup::Found(store) => Ok(store),
        crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )),
        crate::daemon::StoreLookup::Unavailable(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        )),
    }
}

async fn store_for_existing_session_api_error_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    match store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await
    {
        Ok(store) => Ok(store),
        Err(StatusCode::NOT_FOUND) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )),
        Err(StatusCode::INTERNAL_SERVER_ERROR) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "workspace store unavailable".to_string(),
            }),
        )),
        Err(status) => Err((
            status,
            Json(ApiErrorResp {
                error: status.to_string(),
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::subagents::{
        aggregate_subagent_status, legacy_context_window_metric_key, summarize_context_window,
        AgentInitResult,
    };
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
                ctx_core::models::ExecutionEnvironment::Host,
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

    async fn block_workspace_store_for_session(
        data_dir: &tempfile::TempDir,
        state: &Arc<AppState>,
        session: &Session,
    ) {
        state.cleanup_session(session.id).await;
        state
            .core
            .stores
            .evict_workspace(session.workspace_id)
            .await;

        let blocked_workspace_store_dir = data_dir
            .path()
            .join("db")
            .join("workspaces")
            .join(session.workspace_id.0.to_string());
        if let Ok(metadata) = tokio::fs::metadata(&blocked_workspace_store_dir).await {
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(&blocked_workspace_store_dir)
                    .await
                    .unwrap();
            } else {
                tokio::fs::remove_file(&blocked_workspace_store_dir)
                    .await
                    .unwrap();
            }
        }
        tokio::fs::create_dir_all(blocked_workspace_store_dir.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&blocked_workspace_store_dir, b"blocked workspace store")
            .await
            .unwrap();
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

    #[tokio::test]
    async fn existing_session_store_helper_returns_500_when_workspace_store_cannot_open() {
        let (data_dir, state, session) = setup_state().await;
        block_workspace_store_for_session(&data_dir, &state, &session).await;

        let result = store_for_existing_session_status(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected store open failure"),
            Err(status) => assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    #[tokio::test]
    async fn existing_session_store_api_error_helper_returns_500_when_workspace_store_cannot_open()
    {
        let (data_dir, state, session) = setup_state().await;
        block_workspace_store_for_session(&data_dir, &state, &session).await;

        let result = store_for_existing_session_api_error(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected store open failure"),
            Err((status, body)) => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert!(!body.0.error.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn existing_session_write_store_helper_returns_500_when_workspace_store_cannot_open() {
        let (data_dir, state, session) = setup_state().await;
        block_workspace_store_for_session(&data_dir, &state, &session).await;

        let result = store_for_existing_session_status_for_write(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected store open failure"),
            Err(status) => assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    #[tokio::test]
    async fn existing_session_write_store_api_error_helper_returns_500_when_workspace_store_cannot_open(
    ) {
        let (data_dir, state, session) = setup_state().await;
        block_workspace_store_for_session(&data_dir, &state, &session).await;

        let result = store_for_existing_session_api_error_for_write(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected store open failure"),
            Err((status, body)) => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert!(!body.0.error.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn existing_session_store_helpers_return_404_while_workspace_is_deleting() {
        let (_data_dir, state, session) = setup_state().await;
        state
            .core
            .stores
            .begin_workspace_delete(session.workspace_id)
            .await;

        let result = store_for_existing_session_status(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected delete-in-progress session to look missing"),
            Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
        }

        let result = store_for_existing_session_api_error(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected delete-in-progress session to look missing"),
            Err((status, body)) => {
                assert_eq!(status, StatusCode::NOT_FOUND);
                assert_eq!(body.0.error, "session not found");
            }
        }

        let result = store_for_existing_session_status_for_write(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected delete-in-progress session to look missing"),
            Err(status) => assert_eq!(status, StatusCode::NOT_FOUND),
        }

        let result = store_for_existing_session_api_error_for_write(&state, session.id).await;
        match result {
            Ok(_) => panic!("expected delete-in-progress session to look missing"),
            Err((status, body)) => {
                assert_eq!(status, StatusCode::NOT_FOUND);
                assert_eq!(body.0.error, "session not found");
            }
        }

        state
            .core
            .stores
            .finish_workspace_delete(session.workspace_id)
            .await;
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

    #[test]
    fn summarize_context_window_accepts_canonical_metrics() {
        let metrics = serde_json::json!({
            "context_tokens_estimate": 40,
            "context_window_tokens": 100,
            "remaining_tokens_estimate": 60,
            "remaining_fraction": 0.6,
        });

        let summary =
            summarize_context_window(&metrics).expect("expected canonical metrics to parse");
        assert_eq!(summary.total, 100);
        assert_eq!(summary.used, 40);
        assert_eq!(summary.remaining, 60);
        assert!((summary.utilization - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn summarize_context_window_rejects_legacy_alias_metrics() {
        let legacy = serde_json::json!({
            "context_window": 100,
            "total_tokens": 40,
            "remaining_tokens": 60,
        });
        assert!(summarize_context_window(&legacy).is_none());
    }

    #[test]
    fn legacy_context_window_metric_key_detects_first_legacy_key() {
        let legacy = serde_json::json!({
            "context_window": 100,
            "remaining_tokens": 60,
        });
        assert_eq!(
            legacy_context_window_metric_key(&legacy),
            Some("context_window")
        );
    }

    #[test]
    fn legacy_context_window_metric_key_returns_none_for_canonical_shape() {
        let canonical = serde_json::json!({
            "context_tokens_estimate": 40,
            "context_window_tokens": 100,
            "remaining_tokens_estimate": 60,
        });
        assert_eq!(legacy_context_window_metric_key(&canonical), None);
    }
}
