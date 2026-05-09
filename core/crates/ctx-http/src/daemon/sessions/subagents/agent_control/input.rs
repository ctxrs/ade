use super::super::child_runs::enqueue_subagent_prompt;
use super::super::*;

pub(crate) async fn send_input(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: SendInputReq,
) -> ApiResult<SendInputResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    let message = req.message.trim().to_string();
    if message.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "message is required",
        ));
    }

    let interrupt = req.interrupt.unwrap_or(false);
    if interrupt {
        send_scheduler_interrupt(&state, &child).await;
    }

    let persisted = enqueue_subagent_prompt(&state, &child, message).await?;
    let detail = build_enqueued_agent_detail(&state, &parent, &child, &persisted).await;
    Ok(SendInputResp {
        agent: detail,
        queued_run_id: encode_run_ref(persisted.run_id),
        delivery: agent_delivery_label(&persisted.saved_message.delivery).to_string(),
    })
}

pub(crate) async fn archive_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: ArchiveAgentReq,
) -> ApiResult<ArchiveAgentResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    let latest_turn = store
        .get_latest_turn_for_session(child.id)
        .await
        .map_err(internal_api_error)?;
    if latest_turn
        .as_ref()
        .is_some_and(|turn| is_active_turn_status(&turn.status))
    {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "cannot archive agent while it has active or queued work; wait or interrupt first",
        ));
    }

    let archived = store
        .archive_subagent_session(parent.id, child.id)
        .await
        .map_err(internal_api_error)?;
    if !archived {
        return Err(api_error(SubagentErrorKind::NotFound, "agent not found"));
    }
    state
        .workspaces
        .workspace_active_snapshot
        .remove_subagent_session_from_active_task(child.workspace_id, child.task_id, child.id)
        .await;
    state.cleanup_session(child.id).await;
    let cleanup_failed = cleanup_archived_subagent_worktree(&state, &store, &parent, &child).await;

    Ok(ArchiveAgentResp {
        agent_id: encode_agent_ref(child.id),
        task_label: child.title.trim().to_string(),
        archived: true,
        cleanup_failed,
    })
}

pub(crate) async fn interrupt_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: InterruptAgentReq,
) -> ApiResult<InterruptAgentResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.sessions.provider_inactivity_timeout().await;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    send_scheduler_interrupt(&state, &child).await;
    let detail = build_agent_detail(&state, &store, &parent, &child, inactivity_timeout).await?;
    Ok(InterruptAgentResp { agent: detail })
}

async fn send_scheduler_interrupt(state: &Arc<AppState>, child: &ctx_core::models::Session) {
    let tx = state.ensure_scheduler(child.clone()).await;
    let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
    let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;
}
