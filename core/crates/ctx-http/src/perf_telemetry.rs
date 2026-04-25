use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use chrono::{DateTime, Utc};
use opentelemetry::trace::{Span, SpanBuilder, SpanKind, TraceId, Tracer};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_sdk::trace::Span as SdkSpan;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::logs;

mod aggregator;
mod otel;

use aggregator::PerfAggregator;
use otel::OtelRuntime;

const PERF_LOG_PREFIX: &str = "perf-telemetry-";
const PERF_LOG_SUFFIX: &str = ".jsonl";
const PERF_CHANNEL_SEND_TIMEOUT: Duration = Duration::from_millis(250);
const DEFAULT_TRACE_SAMPLE_RATE: f64 = 0.01;
const DEFAULT_TRACE_SLOW_MS: u64 = 2000;
const DEFAULT_RETENTION_DAYS: u64 = 14;

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

pub fn perf_log_path_for_date(data_root: &std::path::Path, date: &str) -> std::path::PathBuf {
    logs::logs_dir(data_root).join(format!("{PERF_LOG_PREFIX}{date}{PERF_LOG_SUFFIX}"))
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PerfMetricKind {
    Histogram,
    Counter,
    Gauge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfMetric {
    pub name: String,
    pub kind: PerfMetricKind,
    pub unit: String,
    pub value: f64,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvent {
    pub occurred_at: DateTime<Utc>,
    pub metric: PerfMetric,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerfMetricSummary {
    pub name: String,
    pub kind: PerfMetricKind,
    pub unit: String,
    pub labels: HashMap<String, String>,
    pub run_id: Option<String>,
    pub count: u64,
    pub sum: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerfSummary {
    pub generated_at: String,
    pub window_ms: Option<u64>,
    pub metrics: Vec<PerfMetricSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerfTelemetryStats {
    pub metric_keys: usize,
    pub total_samples: usize,
    pub max_samples: usize,
}

#[derive(Debug, Clone)]
pub struct PerfTelemetryConfig {
    pub remote_enabled: bool,
    pub remote_override: Option<bool>,
    pub otlp_endpoint: Option<String>,
    pub otlp_headers: HashMap<String, String>,
    pub traces_sample_rate: f64,
    pub traces_slow_ms: u64,
    pub local_retention_days: u64,
}

impl PerfTelemetryConfig {
    pub fn from_env() -> Self {
        let remote_override = env_bool("CTX_TELEMETRY_REMOTE_ENABLED");
        let remote_enabled = remote_override.unwrap_or(true);
        let otlp_endpoint = std::env::var("CTX_TELEMETRY_OTLP_ENDPOINT").ok();
        let otlp_headers = std::env::var("CTX_TELEMETRY_OTLP_HEADERS")
            .ok()
            .map(parse_headers)
            .unwrap_or_default();
        let traces_sample_rate = std::env::var("CTX_TELEMETRY_TRACES_SAMPLE_RATE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_TRACE_SAMPLE_RATE);
        let traces_slow_ms = std::env::var("CTX_TELEMETRY_TRACES_SLOW_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TRACE_SLOW_MS);
        let local_retention_days = std::env::var("CTX_TELEMETRY_LOCAL_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_RETENTION_DAYS);
        Self {
            remote_enabled,
            remote_override,
            otlp_endpoint,
            otlp_headers,
            traces_sample_rate,
            traces_slow_ms,
            local_retention_days,
        }
    }

    pub fn effective_remote_enabled(&self) -> bool {
        self.remote_override.unwrap_or(self.remote_enabled)
    }
}

#[derive(Clone)]
pub struct PerfTelemetry {
    tx: mpsc::Sender<PerfCommand>,
    aggregator: Arc<Mutex<PerfAggregator>>,
    config: Arc<Mutex<PerfTelemetryConfig>>,
    otel: Arc<Mutex<Option<Arc<OtelRuntime>>>>,
}

impl PerfTelemetry {
    pub fn new(data_root: std::path::PathBuf) -> Self {
        let config = PerfTelemetryConfig::from_env();
        let aggregator = Arc::new(Mutex::new(PerfAggregator::default()));
        let otel = Arc::new(Mutex::new(otel::build_otel(&config)));
        let (tx, rx) = mpsc::channel(2048);
        let cfg = Arc::new(Mutex::new(config.clone()));
        let agg = aggregator.clone();
        let otel_clone = otel.clone();
        tokio::spawn(async move {
            perf_worker(data_root, rx, cfg, agg, otel_clone).await;
        });
        Self {
            tx,
            aggregator,
            config: Arc::new(Mutex::new(config)),
            otel,
        }
    }

    pub async fn update_remote_enabled(&self, enabled: bool) {
        {
            let mut cfg = lock_or_recover(self.config.as_ref(), "perf config");
            cfg.remote_enabled = enabled;
        }
        let _ = timeout(
            PERF_CHANNEL_SEND_TIMEOUT,
            self.tx.send(PerfCommand::UpdateRemoteEnabled(enabled)),
        )
        .await;
    }

    pub async fn record_metric(
        &self,
        metric: PerfMetric,
        run_id: Option<String>,
        trace_id: Option<String>,
        span_id: Option<String>,
    ) {
        if let Ok(mut agg) = self.aggregator.lock() {
            agg.record(&metric, run_id.as_deref());
        }
        let event = PerfEvent {
            occurred_at: Utc::now(),
            metric,
            run_id,
            trace_id,
            span_id,
        };
        let _ = timeout(
            PERF_CHANNEL_SEND_TIMEOUT,
            self.tx.send(PerfCommand::Event(event)),
        )
        .await;
    }

    pub fn export_remote_metric(&self, metric: PerfMetric) {
        let cfg = lock_or_recover(self.config.as_ref(), "perf config").clone();
        if !cfg.effective_remote_enabled() {
            return;
        }
        let otel = lock_or_recover(self.otel.as_ref(), "perf otel").clone();
        let Some(otel) = otel else {
            return;
        };
        otel::export_metric(&otel, &metric, &cfg);
    }

    pub fn start_span(
        &self,
        name: &str,
        kind: SpanKind,
        parent: Option<OtelContext>,
        attributes: Vec<KeyValue>,
    ) -> PerfSpan {
        let cfg = lock_or_recover(self.config.as_ref(), "perf config").clone();
        if !cfg.effective_remote_enabled() {
            return PerfSpan::empty();
        }
        let otel = lock_or_recover(self.otel.as_ref(), "perf otel").clone();
        let Some(otel) = otel else {
            return PerfSpan::empty();
        };
        let tracer = otel.tracer();
        let mut builder = SpanBuilder::from_name(name.to_string());
        builder.span_kind = Some(kind);
        builder.start_time = Some(SystemTime::now());
        builder.attributes = Some(attributes);
        let parent = parent.unwrap_or_else(OtelContext::current);
        let span = tracer.build_with_context(builder, &parent);
        let span_ctx = span.span_context().clone();
        let trace_id = if span_ctx.is_valid() {
            Some(span_ctx.trace_id())
        } else {
            None
        };
        let span_id = if span_ctx.is_valid() {
            Some(span_ctx.span_id())
        } else {
            None
        };
        PerfSpan {
            span: Some(span),
            started_at: Instant::now(),
            trace_id,
            span_id,
        }
    }

    pub fn finish_span(
        &self,
        span: PerfSpan,
        status: Option<String>,
        success: Option<bool>,
        extra_attributes: Vec<KeyValue>,
    ) -> (Option<String>, Option<String>) {
        let cfg = lock_or_recover(self.config.as_ref(), "perf config").clone();
        let duration_ms = span.started_at.elapsed().as_millis() as u64;
        let mut trace_id = None;
        let mut span_id = None;
        if let Some(mut s) = span.span {
            if let Some(status) = status.clone() {
                s.set_attribute(KeyValue::new("ctx.status", status));
            }
            if let Some(success) = success {
                s.set_attribute(KeyValue::new("ctx.success", success));
            }
            s.set_attribute(KeyValue::new("ctx.duration_ms", duration_ms as i64));
            for attr in extra_attributes.iter().cloned() {
                s.set_attribute(attr);
            }
            s.end_with_timestamp(SystemTime::now());
            if let Some(id) = span.trace_id {
                trace_id = Some(otel::trace_id_to_string(id));
            }
            if let Some(id) = span.span_id {
                span_id = Some(otel::span_id_to_string(id));
            }
        }

        let should_force = duration_ms >= cfg.traces_slow_ms || success == Some(false);
        if should_force && cfg.effective_remote_enabled() {
            if let Some(otel) = lock_or_recover(self.otel.as_ref(), "perf otel").clone() {
                let tracer = otel.slow_tracer();
                let mut builder = SpanBuilder::from_name("slow_trace".to_string());
                builder.span_kind = Some(SpanKind::Internal);
                builder.start_time = Some(SystemTime::now() - Duration::from_millis(duration_ms));
                builder.attributes = Some(extra_attributes);
                let mut span = tracer.build(builder);
                span.set_attribute(KeyValue::new("ctx.slow_trace", true));
                if let Some(status) = status {
                    span.set_attribute(KeyValue::new("ctx.status", status));
                }
                if let Some(success) = success {
                    span.set_attribute(KeyValue::new("ctx.success", success));
                }
                span.set_attribute(KeyValue::new("ctx.duration_ms", duration_ms as i64));
                span.end_with_timestamp(SystemTime::now());
            }
        }

        (trace_id, span_id)
    }

    pub fn extract_trace_context(&self, headers: &axum::http::HeaderMap) -> OtelContext {
        otel::extract_trace_context(headers)
    }

    pub fn summary(
        &self,
        metric_name: Option<&str>,
        run_id: Option<&str>,
        window_ms: Option<u64>,
        limit: Option<usize>,
    ) -> PerfSummary {
        let metrics = self
            .aggregator
            .lock()
            .map(|agg| agg.summary(metric_name, run_id, window_ms, limit))
            .unwrap_or_default();
        PerfSummary {
            generated_at: Utc::now().to_rfc3339(),
            window_ms,
            metrics,
        }
    }

    pub fn stats(&self) -> PerfTelemetryStats {
        let agg = lock_or_recover(self.aggregator.as_ref(), "perf aggregator");
        agg.stats()
    }
}

pub struct PerfSpan {
    span: Option<SdkSpan>,
    started_at: Instant,
    trace_id: Option<TraceId>,
    span_id: Option<opentelemetry::trace::SpanId>,
}

impl PerfSpan {
    fn empty() -> Self {
        Self {
            span: None,
            started_at: Instant::now(),
            trace_id: None,
            span_id: None,
        }
    }
}

#[derive(Debug)]
enum PerfCommand {
    Event(PerfEvent),
    UpdateRemoteEnabled(bool),
}

async fn perf_worker(
    data_root: std::path::PathBuf,
    mut rx: mpsc::Receiver<PerfCommand>,
    config: Arc<Mutex<PerfTelemetryConfig>>,
    _aggregator: Arc<Mutex<PerfAggregator>>,
    otel: Arc<Mutex<Option<Arc<OtelRuntime>>>>,
) {
    let mut last_cleanup = None::<String>;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            PerfCommand::Event(event) => {
                let cfg = lock_or_recover(config.as_ref(), "perf config").clone();
                if let Err(err) = append_local_log(&data_root, &event).await {
                    tracing::warn!("failed to append perf telemetry log: {err}");
                }
                if cfg.local_retention_days > 0 {
                    let today = Utc::now().format("%Y-%m-%d").to_string();
                    if last_cleanup.as_deref() != Some(&today) {
                        let _ = cleanup_old_logs(&data_root, cfg.local_retention_days).await;
                        last_cleanup = Some(today);
                    }
                }
                if cfg.effective_remote_enabled() {
                    if let Some(runtime) = lock_or_recover(otel.as_ref(), "perf otel").clone() {
                        otel::export_metric(&runtime, &event.metric, &cfg);
                    }
                }
            }
            PerfCommand::UpdateRemoteEnabled(enabled) => {
                if let Ok(mut cfg) = config.lock() {
                    cfg.remote_enabled = enabled;
                }
            }
        }
    }
}

fn parse_headers(raw: String) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in raw.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
}

async fn append_local_log(data_root: &std::path::Path, event: &PerfEvent) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    tokio::fs::create_dir_all(&dir).await.ok();
    let date = event.occurred_at.format("%Y-%m-%d").to_string();
    let path = dir.join(format!("{PERF_LOG_PREFIX}{date}{PERF_LOG_SUFFIX}"));
    let line = serde_json::to_string(event)?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    use tokio::io::AsyncWriteExt;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn cleanup_old_logs(data_root: &std::path::Path, retention_days: u64) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    let mut entries = tokio::fs::read_dir(&dir).await?;
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(PERF_LOG_PREFIX) || !file_name.ends_with(PERF_LOG_SUFFIX) {
            continue;
        }
        let date = file_name
            .trim_start_matches(PERF_LOG_PREFIX)
            .trim_end_matches(PERF_LOG_SUFFIX);
        let Ok(date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        let naive = date.and_hms_opt(0, 0, 0).unwrap_or_default();
        let date = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
        if date < cutoff {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}
