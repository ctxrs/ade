use super::child_runs::enqueue_subagent_prompt;
use super::*;

pub(crate) async fn spawn_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: SpawnAgentReq,
) -> ApiResult<SpawnAgentResp> {
    let task_label = req.task_label.trim().to_string();
    if task_label.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "task_label is required",
        ));
    }
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "prompt is required",
        ));
    }

    let spawned_children = init_subagents(
        state,
        parent_id,
        AgentInitReq {
            tool_call_id: req.tool_call_id,
            response_mode: None,
            worktree: req.worktree,
            agents: vec![crate::api::sessions::AgentInitItem {
                prompt,
                label: Some(task_label.clone()),
                harness: req.harness,
                model: req.model,
                reasoning_effort: req.reasoning_effort,
            }],
        },
    )
    .await?;
    let spawned = spawned_children
        .into_iter()
        .next()
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "spawned agent not found"))?;
    Ok(SpawnAgentResp {
        agent: build_spawned_agent_detail(&spawned),
    })
}

pub(crate) async fn list_agents(
    state: Arc<AppState>,
    parent_id: SessionId,
) -> ApiResult<Vec<AgentSummary>> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let subs = store
        .list_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;
    let mut agents = Vec::with_capacity(subs.len());
    for sub in subs {
        let (summary, _latest_turn) =
            build_agent_summary(&store, sub.id, &sub.title, inactivity_timeout).await?;
        agents.push(summary);
    }
    Ok(agents)
}

pub(crate) async fn get_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: GetAgentReq,
) -> ApiResult<GetAgentResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    let detail = build_agent_detail(&state, &store, &parent, &child, inactivity_timeout).await?;
    Ok(GetAgentResp { agent: detail })
}

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
        let tx = state.ensure_scheduler(child.clone()).await;
        let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
        let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;
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
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    let tx = state.ensure_scheduler(child.clone()).await;
    let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
    let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;
    let detail = build_agent_detail(&state, &store, &parent, &child, inactivity_timeout).await?;
    Ok(InterruptAgentResp { agent: detail })
}

pub(crate) async fn wait_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: WaitAgentReq,
) -> ApiResult<WaitAgentResp> {
    let agent_ids = normalize_wait_agent_ids(&req)?;
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let targets = collect_wait_targets(&store, &parent, &agent_ids).await?;
    let mode = parse_wait_mode(req.mode.as_deref())?;
    let until = parse_wait_until(req.until.as_deref())?;
    if req.since_seq.is_some() && targets.len() != 1 {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "since_seq is only supported with a single agent_id",
        ));
    }

    let timeout_ms = req.timeout_ms.unwrap_or(30_000);
    let mut details = Vec::with_capacity(targets.len());
    for target in &targets {
        details
            .push(build_agent_detail(&state, &store, &parent, target, inactivity_timeout).await?);
    }

    let mut thresholds = HashMap::new();
    match until {
        AgentWaitUntil::Terminal => {}
        AgentWaitUntil::Update => {
            if let Some(since_seq) = req.since_seq {
                thresholds.insert(details[0].agent.agent_id.clone(), since_seq);
            } else {
                for detail in &details {
                    thresholds.insert(detail.agent.agent_id.clone(), detail.agent.last_event_seq);
                }
            }
        }
    }

    if wait_predicate_satisfied(&details, mode, until, &thresholds) {
        return Ok(WaitAgentResp {
            wait_status: "matched".to_string(),
            mode: mode.as_str().to_string(),
            until: until.as_str().to_string(),
            results: details,
        });
    }
    if timeout_ms == 0 {
        return Ok(WaitAgentResp {
            wait_status: "timeout".to_string(),
            mode: mode.as_str().to_string(),
            until: until.as_str().to_string(),
            results: details,
        });
    }

    let started_at = Instant::now();
    while started_at.elapsed() < Duration::from_millis(timeout_ms) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        details.clear();
        for target in &targets {
            details.push(
                build_agent_detail(&state, &store, &parent, target, inactivity_timeout).await?,
            );
        }
        if wait_predicate_satisfied(&details, mode, until, &thresholds) {
            return Ok(WaitAgentResp {
                wait_status: "matched".to_string(),
                mode: mode.as_str().to_string(),
                until: until.as_str().to_string(),
                results: details,
            });
        }
    }

    Ok(WaitAgentResp {
        wait_status: "timeout".to_string(),
        mode: mode.as_str().to_string(),
        until: until.as_str().to_string(),
        results: details,
    })
}
