use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use chrono::{DateTime, Utc};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, Meter, MeterProvider, Unit};
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::{Span, SpanBuilder, SpanKind, TraceId, Tracer};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{Config as TraceConfig, Sampler, Span as SdkSpan};
use opentelemetry_sdk::{runtime, Resource};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::logs;

const PERF_LOG_PREFIX: &str = "perf-telemetry-";
const PERF_LOG_SUFFIX: &str = ".jsonl";
const PERF_CHANNEL_SEND_TIMEOUT: Duration = Duration::from_millis(250);

const DEFAULT_TRACE_SAMPLE_RATE: f64 = 0.01;
const DEFAULT_TRACE_SLOW_MS: u64 = 2000;
const DEFAULT_RETENTION_DAYS: u64 = 14;

pub fn perf_log_path_for_date(data_root: &std::path::Path, date: &str) -> std::path::PathBuf {
    logs::logs_dir(data_root).join(format!("{}{}{}", PERF_LOG_PREFIX, date, PERF_LOG_SUFFIX))
}
const REMOTE_LABEL_ALLOWLIST: &[&str] = &[
    "endpoint",
    "method",
    "status",
    "success",
    "provider_id",
    "model_id",
    "env_target",
    "source",
    "event",
];

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
        let otel = Arc::new(Mutex::new(build_otel(&config)));
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
            let mut cfg = self.config.lock().unwrap();
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

    pub fn start_span(
        &self,
        name: &str,
        kind: SpanKind,
        parent: Option<OtelContext>,
        attributes: Vec<KeyValue>,
    ) -> PerfSpan {
        let cfg = self.config.lock().unwrap().clone();
        if !cfg.effective_remote_enabled() {
            return PerfSpan::empty();
        }
        let otel = self.otel.lock().unwrap().clone();
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
        let cfg = self.config.lock().unwrap().clone();
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
                trace_id = Some(trace_id_to_string(id));
            }
            if let Some(id) = span.span_id {
                span_id = Some(span_id_to_string(id));
            }
        }

        let should_force = duration_ms >= cfg.traces_slow_ms || success == Some(false);
        if should_force && cfg.effective_remote_enabled() {
            if let Some(otel) = self.otel.lock().unwrap().clone() {
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
        let extractor = HeaderExtractor { headers };
        global::get_text_map_propagator(|prop| prop.extract(&extractor))
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

struct PerfAggregator {
    metrics: HashMap<MetricKey, MetricWindow>,
}

impl Default for PerfAggregator {
    fn default() -> Self {
        Self {
            metrics: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MetricKey {
    name: String,
    kind: PerfMetricKind,
    unit: String,
    labels: Vec<(String, String)>,
    run_id: Option<String>,
}

struct MetricWindow {
    samples: Vec<MetricSample>,
    count: u64,
    sum: f64,
}

struct MetricSample {
    value: f64,
    collected_at: Instant,
}

impl PerfAggregator {
    fn record(&mut self, metric: &PerfMetric, run_id: Option<&str>) {
        let mut labels: Vec<(String, String)> = metric
            .labels
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        labels.sort_by(|a, b| a.0.cmp(&b.0));
        let key = MetricKey {
            name: metric.name.clone(),
            kind: metric.kind.clone(),
            unit: metric.unit.clone(),
            labels,
            run_id: run_id.map(|v| v.to_string()),
        };
        let entry = self.metrics.entry(key).or_insert(MetricWindow {
            samples: Vec::new(),
            count: 0,
            sum: 0.0,
        });
        entry.count += 1;
        entry.sum += metric.value;
        entry.samples.push(MetricSample {
            value: metric.value,
            collected_at: Instant::now(),
        });
        const MAX_SAMPLES: usize = 2048;
        if entry.samples.len() > MAX_SAMPLES {
            let overflow = entry.samples.len() - MAX_SAMPLES;
            entry.samples.drain(0..overflow);
        }
    }

    fn summary(
        &self,
        metric_name: Option<&str>,
        run_id: Option<&str>,
        window_ms: Option<u64>,
        limit: Option<usize>,
    ) -> Vec<PerfMetricSummary> {
        let now = Instant::now();
        let window = window_ms.map(Duration::from_millis);
        let mut out = Vec::new();
        for (key, windowed) in &self.metrics {
            if let Some(name) = metric_name {
                if key.name != name {
                    continue;
                }
            }
            if let Some(run_id) = run_id {
                if key.run_id.as_deref() != Some(run_id) {
                    continue;
                }
            }
            let mut values: Vec<f64> = windowed
                .samples
                .iter()
                .filter(|s| match window {
                    Some(w) => now.duration_since(s.collected_at) <= w,
                    None => true,
                })
                .map(|s| s.value)
                .collect();
            if values.is_empty() {
                continue;
            }
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let count = values.len() as u64;
            let sum = values.iter().sum::<f64>();
            let min = values.first().copied();
            let max = values.last().copied();
            let (p50, p95, p99) = if matches!(key.kind, PerfMetricKind::Histogram) {
                (
                    percentile(&values, 0.50),
                    percentile(&values, 0.95),
                    percentile(&values, 0.99),
                )
            } else {
                (None, None, None)
            };
            let labels = key.labels.iter().cloned().collect();
            out.push(PerfMetricSummary {
                name: key.name.clone(),
                kind: key.kind.clone(),
                unit: key.unit.clone(),
                labels,
                run_id: key.run_id.clone(),
                count,
                sum,
                min,
                max,
                p50,
                p95,
                p99,
            });
        }
        if let Some(limit) = limit {
            out.truncate(limit);
        }
        out
    }
}

struct OtelRuntime {
    tracer: opentelemetry_sdk::trace::Tracer,
    slow_tracer: opentelemetry_sdk::trace::Tracer,
    meter: Meter,
    _meter_provider: SdkMeterProvider,
    registry: Mutex<MetricRegistry>,
}

impl OtelRuntime {
    fn tracer(&self) -> &opentelemetry_sdk::trace::Tracer {
        &self.tracer
    }

    fn slow_tracer(&self) -> &opentelemetry_sdk::trace::Tracer {
        &self.slow_tracer
    }
}

#[derive(Default)]
struct MetricRegistry {
    histograms: HashMap<String, Histogram<f64>>,
    counters: HashMap<String, Counter<u64>>,
}

impl MetricRegistry {
    fn histogram(&mut self, meter: &Meter, name: &str, unit: &str) -> Histogram<f64> {
        if let Some(h) = self.histograms.get(name) {
            return h.clone();
        }
        let h = meter
            .f64_histogram(name.to_string())
            .with_unit(Unit::new(unit.to_string()))
            .init();
        self.histograms.insert(name.to_string(), h.clone());
        h
    }

    fn counter(&mut self, meter: &Meter, name: &str, unit: &str) -> Counter<u64> {
        if let Some(c) = self.counters.get(name) {
            return c.clone();
        }
        let c = meter
            .u64_counter(name.to_string())
            .with_unit(Unit::new(unit.to_string()))
            .init();
        self.counters.insert(name.to_string(), c.clone());
        c
    }
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
                let cfg = config.lock().unwrap().clone();
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
                    if let Some(runtime) = otel.lock().unwrap().clone() {
                        export_metric(&runtime, &event.metric, &cfg);
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

fn export_metric(runtime: &OtelRuntime, metric: &PerfMetric, cfg: &PerfTelemetryConfig) {
    let labels = filter_labels(&metric.labels, cfg);
    let attrs = labels_to_kvs(&labels);
    let mut registry = runtime.registry.lock().unwrap();
    match metric.kind {
        PerfMetricKind::Histogram => {
            let hist = registry.histogram(&runtime.meter, &metric.name, &metric.unit);
            hist.record(metric.value, &attrs);
        }
        PerfMetricKind::Counter => {
            let counter = registry.counter(&runtime.meter, &metric.name, &metric.unit);
            counter.add(metric.value as u64, &attrs);
        }
        PerfMetricKind::Gauge => {}
    }
}

fn build_otel(cfg: &PerfTelemetryConfig) -> Option<Arc<OtelRuntime>> {
    let endpoint = cfg.otlp_endpoint.as_ref()?;
    let resource = Resource::new(vec![
        KeyValue::new("service.name", "ctx-daemon"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
        KeyValue::new("os.type", std::env::consts::OS.to_string()),
        KeyValue::new("host.arch", std::env::consts::ARCH.to_string()),
    ]);

    global::set_text_map_propagator(TraceContextPropagator::new());

    let trace_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint)
        .with_headers(cfg.otlp_headers.clone());

    let trace_config = TraceConfig::default()
        .with_resource(resource.clone())
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            cfg.traces_sample_rate,
        ))));
    let tracer = match opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(trace_exporter)
        .with_trace_config(trace_config)
        .install_batch(runtime::Tokio)
    {
        Ok(tracer) => tracer,
        Err(err) => {
            tracing::warn!("failed to initialize OTLP trace exporter: {err}");
            return None;
        }
    };

    let slow_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint)
        .with_headers(cfg.otlp_headers.clone());
    let slow_config = TraceConfig::default()
        .with_resource(resource.clone())
        .with_sampler(Sampler::AlwaysOn);
    let slow_tracer = match opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(slow_exporter)
        .with_trace_config(slow_config)
        .install_batch(runtime::Tokio)
    {
        Ok(tracer) => tracer,
        Err(err) => {
            tracing::warn!("failed to initialize OTLP slow trace exporter: {err}");
            return None;
        }
    };

    let metric_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint)
        .with_headers(cfg.otlp_headers.clone());
    let meter_provider = match opentelemetry_otlp::new_pipeline()
        .metrics(runtime::Tokio)
        .with_exporter(metric_exporter)
        .with_resource(resource)
        .build()
    {
        Ok(provider) => provider,
        Err(err) => {
            tracing::warn!("failed to initialize OTLP metrics exporter: {err}");
            return None;
        }
    };
    let meter = meter_provider.meter("ctx-daemon");

    Some(Arc::new(OtelRuntime {
        tracer,
        slow_tracer,
        meter,
        _meter_provider: meter_provider,
        registry: Mutex::new(MetricRegistry::default()),
    }))
}

fn filter_labels(
    labels: &HashMap<String, String>,
    cfg: &PerfTelemetryConfig,
) -> HashMap<String, String> {
    if !cfg.effective_remote_enabled() {
        return HashMap::new();
    }
    let mut out = HashMap::new();
    for key in REMOTE_LABEL_ALLOWLIST {
        if let Some(value) = labels.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    out
}

fn labels_to_kvs(labels: &HashMap<String, String>) -> Vec<KeyValue> {
    labels
        .iter()
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .collect()
}

fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values.get(idx).copied()
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
    std::env::var(key).ok().and_then(|v| {
        let v = v.trim().to_ascii_lowercase();
        match v.as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    })
}

async fn append_local_log(data_root: &std::path::Path, event: &PerfEvent) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    tokio::fs::create_dir_all(&dir).await.ok();
    let date = event.occurred_at.format("%Y-%m-%d").to_string();
    let path = dir.join(format!("{}{}{}", PERF_LOG_PREFIX, date, PERF_LOG_SUFFIX));
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

fn trace_id_to_string(id: TraceId) -> String {
    format!("{:032x}", id)
}

fn span_id_to_string(id: opentelemetry::trace::SpanId) -> String {
    format!("{:016x}", id)
}

struct HeaderExtractor<'a> {
    headers: &'a axum::http::HeaderMap,
}

impl<'a> Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.headers.keys().map(|k| k.as_str()).collect()
    }
}
