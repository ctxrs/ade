use super::super::*;

use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_session_tools::interrupt_telemetry::{metric_labels, InterruptTelemetryContext};

pub(crate) async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Cancel).await;
    Ok(StatusCode::OK)
}

pub(crate) async fn interrupt_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let request_started = std::time::Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_root_kind = match store.get_worktree(session.worktree_id).await {
        Ok(Some(worktree)) if worktree.vcs_ref.is_some() || worktree.git_branch.is_some() => {
            "worktree"
        }
        Ok(Some(_)) => "workspace_root",
        _ => "unknown",
    };
    let provider_id = session.provider_id.clone();
    let model_id = session.model_id.clone();
    let execution_environment = session.execution_environment;
    let tx = state.ensure_scheduler(session).await;
    let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
    let _ = tx
        .send(SchedulerCommand::Interrupt(interrupt.clone()))
        .await;
    let dispatch_ms = request_started.elapsed().as_millis() as u64;
    let metric = PerfMetric {
        name: "scheduler.interrupt_http_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: dispatch_ms as f64,
        labels: metric_labels(
            &provider_id,
            &model_id,
            execution_environment.as_str(),
            session_root_kind,
            "http_dispatch",
        ),
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
    tracing::info!(
        session_id = %session_id.0,
        interrupt_id = %interrupt.interrupt_id(),
        provider_id = %provider_id,
        model_id = %model_id,
        dispatch_ms,
        "session interrupt dispatched"
    );
    Ok(StatusCode::OK)
}
