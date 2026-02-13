use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct TelemetrySummaryQuery {
    metric: Option<String>,
    run_id: Option<String>,
    window_ms: Option<u64>,
    limit: Option<u32>,
}

pub(super) async fn get_telemetry_summary(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TelemetrySummaryQuery>,
) -> Result<Json<crate::perf_telemetry::PerfSummary>, StatusCode> {
    let limit = q.limit.map(|v| v as usize);
    let summary = state.telemetry.perf_telemetry.summary(
        q.metric.as_deref(),
        q.run_id.as_deref(),
        q.window_ms,
        limit,
    );
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
pub(super) struct TelemetryExportQuery {
    date: Option<String>,
}

pub(super) async fn export_telemetry(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TelemetryExportQuery>,
) -> Result<Response, StatusCode> {
    let date = q
        .date
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let path = crate::perf_telemetry::perf_log_path_for_date(&state.core.data_root, &date);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    Ok(resp)
}

#[derive(Debug, Deserialize)]
pub(super) struct ClientTelemetryMetric {
    name: String,
    kind: PerfMetricKind,
    unit: String,
    value: f64,
    labels: Option<HashMap<String, String>>,
    run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClientTelemetryBatch {
    events: Vec<ClientTelemetryMetric>,
}

pub(super) async fn post_client_telemetry(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<ClientTelemetryBatch>,
) -> Result<StatusCode, StatusCode> {
    for event in batch.events {
        let mut labels = event.labels.unwrap_or_default();
        labels.insert("source".to_string(), "client".to_string());
        let metric = PerfMetric {
            name: event.name,
            kind: event.kind,
            unit: event.unit,
            value: event.value,
            labels,
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(metric, event.run_id, None, None)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
