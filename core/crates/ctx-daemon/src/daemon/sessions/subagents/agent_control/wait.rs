use super::super::*;

pub async fn wait_agent(
    state: Arc<DaemonState>,
    parent_id: SessionId,
    req: WaitAgentReq,
) -> ApiResult<WaitAgentResp> {
    let agent_ids = normalize_wait_agent_ids(req.agent_id.as_deref(), req.agent_ids.as_deref())
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let targets = collect_wait_targets(&store, &parent, &agent_ids).await?;
    let mode = parse_wait_mode(req.mode.as_deref())
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    let until = parse_wait_until(req.until.as_deref())
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    if req.since_seq.is_some() && targets.len() != 1 {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "since_seq is only supported with a single agent_id",
        ));
    }

    let timeout_ms = req.timeout_ms.unwrap_or(30_000);
    let mut details =
        collect_wait_details(&state, &store, &parent, &targets, inactivity_timeout).await?;
    let thresholds = wait_update_thresholds(&details, until, req.since_seq);

    if wait_predicate_satisfied(&agent_wait_details(&details), mode, until, &thresholds) {
        return Ok(wait_response("matched", mode, until, details));
    }
    if timeout_ms == 0 {
        return Ok(wait_response("timeout", mode, until, details));
    }

    let started_at = Instant::now();
    while started_at.elapsed() < Duration::from_millis(timeout_ms) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        details =
            collect_wait_details(&state, &store, &parent, &targets, inactivity_timeout).await?;
        if wait_predicate_satisfied(&agent_wait_details(&details), mode, until, &thresholds) {
            return Ok(wait_response("matched", mode, until, details));
        }
    }

    Ok(wait_response("timeout", mode, until, details))
}

async fn collect_wait_details(
    state: &Arc<DaemonState>,
    store: &ctx_store::store::Store,
    parent: &ctx_core::models::Session,
    targets: &[ctx_core::models::Session],
    inactivity_timeout: Duration,
) -> ApiResult<Vec<AgentDetail>> {
    let mut details = Vec::with_capacity(targets.len());
    for target in targets {
        details.push(build_agent_detail(state, store, parent, target, inactivity_timeout).await?);
    }
    Ok(details)
}

fn wait_update_thresholds(
    details: &[AgentDetail],
    until: AgentWaitUntil,
    since_seq: Option<i64>,
) -> HashMap<String, i64> {
    let mut thresholds = HashMap::new();
    match until {
        AgentWaitUntil::Terminal => {}
        AgentWaitUntil::Update => {
            if let Some(since_seq) = since_seq {
                thresholds.insert(details[0].agent.agent_id.clone(), since_seq);
            } else {
                for detail in details {
                    thresholds.insert(detail.agent.agent_id.clone(), detail.agent.last_event_seq);
                }
            }
        }
    }
    thresholds
}

fn wait_response(
    wait_status: &str,
    mode: ctx_session_service::subagents::AgentWaitMode,
    until: AgentWaitUntil,
    results: Vec<AgentDetail>,
) -> WaitAgentResp {
    WaitAgentResp {
        wait_status: wait_status.to_string(),
        mode: mode.as_str().to_string(),
        until: until.as_str().to_string(),
        results,
    }
}

fn agent_wait_details(details: &[AgentDetail]) -> Vec<AgentWaitDetail<'_>> {
    details
        .iter()
        .map(|detail| AgentWaitDetail {
            agent_id: &detail.agent.agent_id,
            has_current_run: detail.agent.current_run_id.is_some(),
            has_latest_result: detail.agent.latest_result_status.is_some(),
            last_event_seq: detail.agent.last_event_seq,
        })
        .collect()
}
