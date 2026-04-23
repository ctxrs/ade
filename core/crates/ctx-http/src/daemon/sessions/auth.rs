use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::daemon::AppState;
use crate::logs;
use crate::order_seq::attach_order_seq;
use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEventType};
use ctx_providers::adapters::{ProviderAdapter, ProviderRunHooks, ProviderSessionRefClaimHook};
use ctx_providers::events::NormalizedEvent;

#[derive(Debug)]
pub(crate) enum SessionAuthError {
    NotFound(&'static str),
    BadRequest(String),
    Internal(String),
    AuthenticationFailed { redacted_message: String },
}

struct PreparedSessionAuth {
    adapter: Arc<dyn ProviderAdapter>,
    workdir: PathBuf,
    provider_env: HashMap<String, String>,
}

fn provider_session_claim_hook(
    store: ctx_store::Store,
    session_id: SessionId,
) -> ProviderSessionRefClaimHook {
    Arc::new(move |claim| {
        let store = store.clone();
        Box::pin(async move {
            if let Some(returned_ref) = claim.returned_provider_session_ref {
                store
                    .claim_session_provider_session_ref(
                        session_id,
                        returned_ref,
                        "provider.session_opened.auth",
                    )
                    .await?;
            }
            Ok(())
        })
    })
}

pub(crate) async fn run_session_authentication(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &Session,
    method_id: Option<String>,
) -> Result<(), SessionAuthError> {
    let prepared = prepare_session_auth_runtime(state, store, session).await?;
    let event_sender = spawn_session_auth_event_sink(Arc::clone(state), store.clone(), session.id);

    append_auth_notice(
        state,
        store,
        session.id,
        serde_json::json!({
            "kind": "auth_started",
            "provider": session.provider_id,
            "method_id": method_id,
        }),
    )
    .await?;

    let result = prepared
        .adapter
        .authenticate_session(
            session.id.0.to_string(),
            prepared.workdir,
            prepared.provider_env,
            method_id,
            event_sender,
            ProviderRunHooks {
                provider_session_ref_claim: Some(provider_session_claim_hook(
                    store.clone(),
                    session.id,
                )),
            },
        )
        .await;

    match result {
        Ok(()) => {
            append_auth_notice(
                state,
                store,
                session.id,
                serde_json::json!({
                    "kind": "auth_finished",
                    "provider": session.provider_id,
                }),
            )
            .await?;
            Ok(())
        }
        Err(error) => {
            let redacted_message = logs::redact_sensitive(&error.to_string());
            append_auth_notice(
                state,
                store,
                session.id,
                serde_json::json!({
                    "kind": "auth_failed",
                    "provider": session.provider_id,
                    "message": redacted_message,
                }),
            )
            .await?;
            Err(SessionAuthError::AuthenticationFailed { redacted_message })
        }
    }
}

async fn prepare_session_auth_runtime(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &Session,
) -> Result<PreparedSessionAuth, SessionAuthError> {
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| SessionAuthError::Internal("failed to load worktree".to_string()))?
        .ok_or(SessionAuthError::NotFound("worktree"))?;
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await
        .map_err(|error| {
            SessionAuthError::Internal(format!("failed to load workspace: {error:#}"))
        })?
        .ok_or(SessionAuthError::NotFound("workspace"))?;
    let resolved_worktree = crate::api::tasks::resolve_existing_worktree_execution(
        state,
        store,
        &workspace,
        worktree.id,
    )
    .await
    .map_err(|error| {
        SessionAuthError::Internal(format!(
            "failed to resolve session worktree execution: {error:#}"
        ))
    })?;
    let execution_environment = resolved_worktree.execution_environment();
    if session.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %session.id.0,
            stored = session.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "session authenticate resolved a different execution_environment than persisted metadata"
        );
    }
    let install_target = crate::execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        execution_environment,
    )
    .await
    .map_err(|error| {
        SessionAuthError::Internal(format!(
            "failed to load workspace execution settings: {error:#}"
        ))
    })?;
    let adapter = crate::daemon::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await;
    let probe_context = crate::provider_launch::probe::provider_auth_context_for_worktree_runtime(
        state.as_ref(),
        &resolved_worktree.worktree,
        &session.provider_id,
    )
    .await
    .map_err(SessionAuthError::BadRequest)?;
    let mut provider_env = probe_context.env;
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    if let Ok(value) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), value);
    }
    if let Ok(value) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), value);
    }
    if session.provider_id == "codex" && !provider_env.contains_key("CODEX_HOME") {
        if let Ok(extra) =
            ctx_provider_accounts::codex_env_for_active_account(&state.core.data_root).await
        {
            for (key, value) in extra {
                provider_env.insert(key, value);
            }
        }
    }
    let adapter_cfg = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    crate::installer::ensure_codex_cli_command_env_for_target(
        &mut provider_env,
        &adapter_cfg,
        &session.provider_id,
        Some(install_target),
    )
    .map_err(|error| {
        SessionAuthError::Internal(format!(
            "failed to resolve codex-cli runtime path: {error:#}"
        ))
    })?;
    crate::mcp_command::configure_runtime_mcp_command(&mut provider_env, &state.core.data_root)
        .map_err(|error| {
            SessionAuthError::Internal(format!("failed to prepare sandbox MCP runtime: {error:#}"))
        })?;

    Ok(PreparedSessionAuth {
        adapter,
        workdir: probe_context.cwd,
        provider_env,
    })
}

fn spawn_session_auth_event_sink(
    state: Arc<AppState>,
    store: ctx_store::Store,
    session_id: SessionId,
) -> mpsc::Sender<NormalizedEvent> {
    let (event_sender, mut event_receiver) = mpsc::channel::<NormalizedEvent>(128);
    tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            let mut event_type = event.event_type.clone();
            let mut payload = event.payload_json.clone();
            if matches!(event.event_type, SessionEventType::Init) {
                if payload.get("crp_session_id").is_some() {
                    state
                        .emit_compat_payload_reject_counter(
                            "sessions.auth_event_init",
                            "crp_session_id",
                            None,
                        )
                        .await;
                }
                if let Some(provider_session_id) = payload
                    .get("provider_session_id")
                    .and_then(serde_json::Value::as_str)
                {
                    if let Err(err) = store
                        .claim_session_provider_session_ref(
                            session_id,
                            provider_session_id.to_string(),
                            "sessions.auth_event_init",
                        )
                        .await
                    {
                        event_type = SessionEventType::Error;
                        payload = serde_json::json!({
                            "message": err.to_string(),
                            "reason": "provider_session_ref_claim_failed",
                            "kind": "provider_session_ref_claim_failed",
                            "details": {
                                "provider_session_id": provider_session_id,
                            },
                        });
                    }
                }
            }
            if payload.is_object() {
                let should_attach = matches!(
                    event.event_type,
                    SessionEventType::UserMessage
                        | SessionEventType::AssistantChunk
                        | SessionEventType::AssistantComplete
                        | SessionEventType::AssistantMessageInserted
                        | SessionEventType::ThoughtChunk
                        | SessionEventType::ToolCall
                        | SessionEventType::ToolCallUpdate
                        | SessionEventType::ToolResult
                ) || (matches!(event.event_type, SessionEventType::Notice)
                    && payload
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|kind| {
                            kind == "reasoning_summary" || kind == "ask_user_question"
                        }));
                if should_attach {
                    let order_seq_state =
                        state.sessions.get_order_seq_state(&store, session_id).await;
                    let mut order_seq_state = order_seq_state.lock().await;
                    attach_order_seq(
                        &mut order_seq_state,
                        &event.event_type,
                        &mut payload,
                        None,
                        0,
                    );
                }
            }
            if let Ok(appended_event) = store
                .append_session_event(session_id, None, None, event_type, payload)
                .await
            {
                state.publish_event(appended_event).await;
            }
        }
    });
    event_sender
}

async fn append_auth_notice(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session_id: SessionId,
    payload: serde_json::Value,
) -> Result<(), SessionAuthError> {
    let event = store
        .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
        .await
        .map_err(|_| SessionAuthError::Internal("failed to append auth event".to_string()))?;
    state.publish_event(event).await;
    Ok(())
}
