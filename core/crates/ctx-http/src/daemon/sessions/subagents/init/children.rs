use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::ids::TurnId;
use ctx_core::models::{
    ExecutionEnvironment, Session, SubagentInvocationChild, VcsKind, Workspace,
};
use ctx_session_service::subagents::SubagentWorktreeSelection;
use ctx_session_tools::model_resolution::{resolve_model_id, ModelCatalog};
use tokio::sync::Mutex;

use crate::api::sessions::AgentInitItem;
use crate::daemon::AppState;
use crate::settings::ExecutionSettings;

use super::super::errors::{api_error, internal_api_error, ApiResult, SubagentErrorKind};
use super::super::request::default_catalog_model_id;
use super::super::worktrees::create_subagent_worktree;
use super::super::{
    dispatch_subagent_prompt, emit_subagent_invocation_notice, persist_subagent_prompt,
    SpawnedChild,
};

#[derive(Clone)]
pub(super) struct SubagentChildInit {
    pub(super) state: Arc<AppState>,
    pub(super) parent: Session,
    pub(super) workspace: Workspace,
    pub(super) model_catalogs: HashMap<String, Option<ModelCatalog>>,
    pub(super) invocation_id: String,
    pub(super) tool_call_id: String,
    pub(super) child_ids: Arc<Mutex<Vec<String>>>,
    pub(super) parent_turn_id: Option<TurnId>,
    pub(super) worktree_selection: SubagentWorktreeSelection,
    pub(super) worktree_plan: Option<(VcsKind, String)>,
    pub(super) parent_effective: ExecutionSettings,
    pub(super) execution_environment: ExecutionEnvironment,
}

pub(super) struct SubagentChildInitItem {
    pub(super) idx: usize,
    pub(super) agent: AgentInitItem,
    pub(super) label: String,
}

pub(super) async fn create_subagent_child(
    init: SubagentChildInit,
    item: SubagentChildInitItem,
) -> ApiResult<SpawnedChild> {
    let store = init
        .state
        .store_for_session(init.parent.id)
        .await
        .map_err(internal_api_error)?;
    let prompt = item.agent.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!("agent {} prompt is required", item.idx + 1),
        ));
    }

    let provider_id = resolve_provider_id(&init, &item).await;
    let catalog = init
        .model_catalogs
        .get(&provider_id)
        .and_then(|value| value.as_ref());
    let fallback_model = fallback_model_id(&init, &item, &provider_id, catalog).await?;
    let resolved = resolve_model_id(
        item.agent.model.as_deref(),
        item.agent.reasoning_effort.as_deref(),
        fallback_model.as_deref(),
        catalog,
    )
    .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

    let prompt_length = prompt.chars().count() as i64;
    let reasoning_effort = resolved.reasoning_effort.clone();
    let (worktree_id, worktree_path) = resolve_child_worktree(&init, &store).await?;

    let session = store
        .create_session_with_reasoning_effort(
            init.parent.task_id,
            init.parent.workspace_id,
            worktree_id,
            init.execution_environment,
            provider_id.clone(),
            resolved.model_id.clone(),
            reasoning_effort.clone(),
            "subagent".into(),
            Some(init.parent.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .map_err(internal_api_error)?;
    index_child_session(&init, &store, &session, &item.label).await;

    let child_created_at = chrono::Utc::now();
    let persisted = persist_subagent_prompt(&init.state, &session, prompt).await?;
    let child_session_id = session.id;
    let child = SubagentInvocationChild {
        invocation_id: init.invocation_id.clone(),
        child_session_id,
        run_id: Some(persisted.run_id),
        position: item.idx as i64,
        status: "running".to_string(),
        label: Some(item.label),
        harness: Some(provider_id),
        model: Some(resolved.full_model_id),
        reasoning_effort,
        prompt_length,
        created_at: child_created_at,
        updated_at: child_created_at,
    };
    store
        .upsert_subagent_invocation_child(child.clone())
        .await
        .map_err(internal_api_error)?;

    let child_ids_snapshot = {
        let mut ids = init.child_ids.lock().await;
        ids.push(child_session_id.0.to_string());
        ids.clone()
    };
    emit_subagent_invocation_notice(
        &init.state,
        init.parent.id,
        init.parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": init.invocation_id.clone(),
            "tool_call_id": init.tool_call_id.clone(),
            "status": "running",
            "child_session_ids": child_ids_snapshot,
        }),
    )
    .await?;
    dispatch_subagent_prompt(&init.state, &session, &persisted.saved_message).await;

    Ok(SpawnedChild {
        child,
        worktree_path,
        last_event_seq: persisted.last_event_seq,
    })
}

async fn resolve_provider_id(init: &SubagentChildInit, item: &SubagentChildInitItem) -> String {
    let harness_defaulted = item.agent.harness.is_none();
    let provider_id = item
        .agent
        .harness
        .as_deref()
        .unwrap_or(&init.parent.provider_id)
        .trim()
        .to_string();
    if harness_defaulted {
        init.state
            .emit_product_fallback_applied_counter(
                "sessions.subagent_init",
                "harness_default_parent",
                None,
            )
            .await;
    }
    provider_id
}

async fn fallback_model_id<'a>(
    init: &'a SubagentChildInit,
    item: &SubagentChildInitItem,
    provider_id: &str,
    catalog: Option<&'a ModelCatalog>,
) -> ApiResult<Option<String>> {
    let fallback_model = if item.agent.model.is_none() {
        if provider_id == init.parent.provider_id {
            Some(init.parent.model_id.clone())
        } else {
            default_catalog_model_id(catalog).map(ToOwned::to_owned)
        }
    } else {
        None
    };
    if item.agent.model.is_none() && fallback_model.is_none() {
        init.state
            .emit_compat_payload_reject_counter(
                "sessions.subagent_init",
                "missing_model_without_default",
                Some(("provider_id", provider_id)),
            )
            .await;
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!("model is required for harness '{provider_id}'"),
        ));
    }
    if item.agent.model.is_none() {
        let fallback = if provider_id == init.parent.provider_id {
            "model_default_parent"
        } else {
            "model_default_catalog"
        };
        init.state
            .emit_product_fallback_applied_counter("sessions.subagent_init", fallback, None)
            .await;
    }
    Ok(fallback_model)
}

async fn resolve_child_worktree(
    init: &SubagentChildInit,
    store: &ctx_store::Store,
) -> ApiResult<(ctx_core::ids::WorktreeId, Option<String>)> {
    match init.worktree_selection {
        SubagentWorktreeSelection::Inherit => Ok((init.parent.worktree_id, None)),
        SubagentWorktreeSelection::New => {
            let (vcs_kind, base_commit_sha) = init
                .worktree_plan
                .clone()
                .ok_or_else(|| api_error(SubagentErrorKind::Internal, "worktree plan missing"))?;
            let worktree = create_subagent_worktree(
                &init.state,
                store,
                &init.workspace,
                init.parent.task_id,
                &base_commit_sha,
                vcs_kind,
                &init.parent_effective,
            )
            .await?;
            Ok((worktree.id, Some(worktree.root_path)))
        }
    }
}

async fn index_child_session(
    init: &SubagentChildInit,
    store: &ctx_store::Store,
    session: &Session,
    label: &str,
) {
    if let Err(error) = init
        .state
        .global_store()
        .upsert_workspace_session_index(session.id, init.parent.workspace_id)
        .await
    {
        tracing::warn!(
            session_id = %session.id.0,
            "failed to update subagent session index: {error:?}"
        );
    }
    if store
        .update_session_title(session.id, label.to_string())
        .await
        .is_err()
    {
        tracing::warn!(session_id = %session.id.0, "failed to set subagent label");
    }
}
