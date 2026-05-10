use super::*;

mod children;
mod invocation;
mod request;
mod spawning;

use children::{create_subagent_child, SubagentChildInit, SubagentChildInitItem};
use invocation::{
    mark_subagent_invocation_failed, start_subagent_invocation, StartedSubagentInvocation,
};
use request::{
    build_subagent_request_agents, prepare_subagent_init_request, PreparedSubagentInitRequest,
};
use spawning::spawn_subagent_completion_tasks;

pub(crate) async fn init_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: AgentInitReq,
) -> ApiResult<Vec<SpawnedChild>> {
    let PreparedSubagentInitRequest {
        agents,
        labels,
        request_json,
        tool_call_id,
        worktree_selection,
    } = prepare_subagent_init_request(&state, &req).await?;

    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let creation_lock = state
        .sessions
        .task_session_creation_lock(parent.task_id)
        .await;
    let _creation_guard = creation_lock.lock().await;
    if parent.parent_session_id.is_some() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "subagents cannot spawn child agents; max depth is {}",
                DEFAULT_MAX_SUBAGENT_DEPTH
            ),
        ));
    }
    let existing_active = store
        .count_active_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;
    if existing_active + agents.len() > DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "max {} active child agents per parent",
                DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT
            ),
        ));
    }
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "workspace not found"))?;

    ensure_requested_labels_available(&store, parent.task_id, &labels).await?;
    let request_agents = build_subagent_request_agents(&agents);
    let provider_ids = collect_provider_ids(&request_agents, &parent.provider_id)
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

    let parent_worktree_execution = crate::api::tasks::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        parent.worktree_id,
    )
    .await
    .map_err(internal_api_error)?;
    let parent_worktree = parent_worktree_execution.worktree.clone();
    let resolved_parent_execution_environment = parent_worktree_execution.execution_environment();
    if parent.execution_environment != resolved_parent_execution_environment {
        tracing::warn!(
            session_id = %parent.id.0,
            stored = parent.execution_environment.as_str(),
            resolved = resolved_parent_execution_environment.as_str(),
            "parent session execution_environment drifted from resolved worktree identity"
        );
    }

    let model_catalogs = load_requested_model_catalogs(
        &state,
        &workspace,
        &provider_ids,
        resolved_parent_execution_environment,
    )
    .await?;
    let worktree_plan =
        plan_subagent_worktree_creation(&state, &parent_worktree, worktree_selection).await?;

    let StartedSubagentInvocation {
        invocation_id,
        tool_call_id,
        parent_turn_id,
    } = start_subagent_invocation(
        &state,
        &store,
        &parent,
        agents.len(),
        Some(request_json),
        tool_call_id.as_deref(),
    )
    .await?;

    let child_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let child_init = SubagentChildInit {
        state: state.clone(),
        parent: parent.clone(),
        workspace: workspace.clone(),
        model_catalogs,
        invocation_id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        child_ids: child_ids.clone(),
        parent_turn_id,
        worktree_selection,
        worktree_plan,
        parent_effective: parent_worktree_execution.effective.clone(),
        execution_environment: resolved_parent_execution_environment,
    };

    let mut futures = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.into_iter().enumerate() {
        let label = labels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("Subagent {}", idx + 1));
        futures.push(create_subagent_child(
            child_init.clone(),
            SubagentChildInitItem { idx, agent, label },
        ));
    }

    let spawned_children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(error) => {
            let child_session_ids = {
                let ids = child_ids.lock().await;
                ids.clone()
            };
            mark_subagent_invocation_failed(
                &state,
                &parent,
                &invocation_id,
                &tool_call_id,
                parent_turn_id,
                &child_session_ids,
            )
            .await;
            return Err(error);
        }
    };

    spawn_subagent_completion_tasks(
        &state,
        &spawned_children,
        invocation_id,
        tool_call_id,
        parent.id,
        parent_turn_id,
        parent.worktree_id,
    );

    Ok(spawned_children)
}
