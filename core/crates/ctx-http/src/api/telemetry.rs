use super::*;
use crate::telemetry::{
    TelemetryDelivery, TelemetryEvent, TelemetryOriginRuntime, TelemetryPlane, TelemetryProperties,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value};

const MAX_SEMANTIC_EVENTS: usize = 200;
const MAX_TELEMETRY_PROPERTY_COUNT: usize = 64;
const MAX_TELEMETRY_KEY_LENGTH: usize = 80;
const MAX_TELEMETRY_STRING_LENGTH: usize = 512;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SemanticTelemetryBatch {
    events: Vec<SemanticTelemetryEventReq>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticTelemetryEventReq {
    event_id: String,
    event_name: String,
    event_version: u32,
    occurred_at: DateTime<Utc>,
    plane: TelemetryPlane,
    delivery: TelemetryDelivery,
    origin_runtime: TelemetryOriginRuntime,
    origin_install_id: String,
    app_version: String,
    os: String,
    arch: String,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    env_target: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    properties: Option<Map<String, Value>>,
}

impl SemanticTelemetryEventReq {
    fn into_event(self) -> Result<TelemetryEvent, StatusCode> {
        let event_id = self.event_id.trim().to_string();
        let event_name = self.event_name.trim().to_string();
        let origin_install_id = self.origin_install_id.trim().to_string();
        let app_version = self.app_version.trim().to_string();
        let os = self.os.trim().to_string();
        let arch = self.arch.trim().to_string();
        if event_id.is_empty()
            || event_name.is_empty()
            || self.event_version == 0
            || origin_install_id.is_empty()
            || app_version.is_empty()
            || os.is_empty()
            || arch.is_empty()
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        Ok(TelemetryEvent {
            event_id,
            event_name,
            event_version: self.event_version,
            occurred_at: self.occurred_at,
            plane: self.plane,
            delivery: self.delivery,
            origin_runtime: self.origin_runtime,
            origin_install_id: Some(origin_install_id),
            app_version,
            os,
            arch,
            surface: normalize_optional_string(self.surface),
            env_target: normalize_optional_string(self.env_target),
            source: normalize_optional_string(self.source),
            properties: sanitize_semantic_properties(self.properties.unwrap_or_default()),
        })
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn sanitize_semantic_properties(raw: Map<String, Value>) -> TelemetryProperties {
    let mut out = TelemetryProperties::new();
    for (key, value) in raw {
        if out.len() >= MAX_TELEMETRY_PROPERTY_COUNT {
            break;
        }
        if key.trim().is_empty() || key.len() > MAX_TELEMETRY_KEY_LENGTH {
            continue;
        }
        if is_forbidden_semantic_property_key(&key) {
            continue;
        }
        let Some(value) = sanitize_semantic_value(value) else {
            continue;
        };
        out.insert(key, value);
    }
    out
}

fn sanitize_semantic_value(value: Value) -> Option<Value> {
    match value {
        Value::Null => Some(Value::Null),
        Value::Bool(value) => Some(Value::Bool(value)),
        Value::Number(value) => {
            if value.is_f64() {
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(Value::from)
            } else {
                Some(Value::Number(value))
            }
        }
        Value::String(value) => Some(Value::String(
            value.chars().take(MAX_TELEMETRY_STRING_LENGTH).collect(),
        )),
        _ => None,
    }
}

fn normalized_property_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_forbidden_semantic_property_key(key: &str) -> bool {
    let normalized = normalized_property_key(key);
    matches!(
        normalized.as_str(),
        "workspaceid" | "taskid" | "sessionid" | "worktreeid" | "runid" | "turnid"
    ) || [
        "prompt",
        "code",
        "filepath",
        "reponame",
        "branch",
        "command",
        "token",
        "secret",
        "apikey",
        "password",
        "authorization",
        "cookie",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
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

pub(super) async fn post_semantic_telemetry(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<SemanticTelemetryBatch>,
) -> Result<StatusCode, StatusCode> {
    if batch.events.len() > MAX_SEMANTIC_EVENTS {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut events = Vec::with_capacity(batch.events.len());
    for event in batch.events {
        events.push(event.into_event()?);
    }
    state.telemetry.telemetry.emit_many(events).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn semantic_event_requires_non_empty_core_fields() {
        let event = SemanticTelemetryEventReq {
            event_id: "   ".to_string(),
            event_name: "renderer_backlog_spike".to_string(),
            event_version: 1,
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            plane: TelemetryPlane::Incident,
            delivery: TelemetryDelivery::Remote,
            origin_runtime: TelemetryOriginRuntime::Web,
            origin_install_id: "install-1".to_string(),
            app_version: "1.2.3".to_string(),
            os: "macos".to_string(),
            arch: "arm64".to_string(),
            surface: Some("desktop".to_string()),
            env_target: None,
            source: None,
            properties: None,
        };

        assert_eq!(event.into_event().unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn semantic_properties_drop_nested_values_and_bound_strings() {
        let mut properties = Map::new();
        properties.insert("ok".to_string(), Value::Bool(true));
        properties.insert("nested".to_string(), serde_json::json!({ "bad": "value" }));
        properties.insert("workspace_id".to_string(), Value::String("raw".to_string()));
        properties.insert("sessionId".to_string(), Value::String("raw".to_string()));
        properties.insert("prompt_body".to_string(), Value::String("raw".to_string()));
        properties.insert("command".to_string(), Value::String("raw".to_string()));
        properties.insert(
            "long".to_string(),
            Value::String("x".repeat(MAX_TELEMETRY_STRING_LENGTH + 20)),
        );
        let event = SemanticTelemetryEventReq {
            event_id: "event-1".to_string(),
            event_name: "worker_patch_apply".to_string(),
            event_version: 2,
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            plane: TelemetryPlane::Incident,
            delivery: TelemetryDelivery::LocalOnly,
            origin_runtime: TelemetryOriginRuntime::Desktop,
            origin_install_id: "install-1".to_string(),
            app_version: "1.2.3".to_string(),
            os: "macos".to_string(),
            arch: "arm64".to_string(),
            surface: Some("desktop".to_string()),
            env_target: Some("host".to_string()),
            source: Some("worker_patch".to_string()),
            properties: Some(properties),
        };

        let event = event.into_event().expect("valid semantic event");
        assert_eq!(event.delivery, TelemetryDelivery::LocalOnly);
        assert_eq!(event.properties.get("ok"), Some(&Value::Bool(true)));
        assert!(!event.properties.contains_key("nested"));
        assert!(!event.properties.contains_key("workspace_id"));
        assert!(!event.properties.contains_key("sessionId"));
        assert!(!event.properties.contains_key("prompt_body"));
        assert!(!event.properties.contains_key("command"));
        assert_eq!(
            event.properties.get("long"),
            Some(&Value::String("x".repeat(MAX_TELEMETRY_STRING_LENGTH)))
        );
    }
}
