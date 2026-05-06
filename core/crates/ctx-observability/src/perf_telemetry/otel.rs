use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, MeterProvider, Unit};
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::{TraceId, TracerProvider};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{
    BatchSpanProcessor, Config as TraceConfig, Sampler, TracerProvider as SdkTracerProvider,
};
use opentelemetry_sdk::{runtime, Resource};
use tokio::time::timeout;

use super::{lock_or_recover, PerfMetric, PerfMetricKind, PerfTelemetryConfig};

const REMOTE_LABEL_ALLOWLIST: &[&str] = &[
    "endpoint",
    "method",
    "status",
    "success",
    "provider_id",
    "model_id",
    "execution_environment",
    "session_root_kind",
    "source",
    "event",
];
static FIRST_REMOTE_EXPORT: AtomicBool = AtomicBool::new(false);

pub(super) struct OtelRuntime {
    tracer: opentelemetry_sdk::trace::Tracer,
    slow_tracer: opentelemetry_sdk::trace::Tracer,
    _trace_provider: opentelemetry_sdk::trace::TracerProvider,
    _slow_trace_provider: opentelemetry_sdk::trace::TracerProvider,
    meter: Meter,
    _meter_provider: SdkMeterProvider,
    registry: Mutex<MetricRegistry>,
}

impl OtelRuntime {
    pub(super) fn tracer(&self) -> &opentelemetry_sdk::trace::Tracer {
        &self.tracer
    }

    pub(super) fn slow_tracer(&self) -> &opentelemetry_sdk::trace::Tracer {
        &self.slow_tracer
    }
}

#[derive(Default)]
struct MetricRegistry {
    histograms: HashMap<String, Histogram<f64>>,
    counters: HashMap<String, Counter<u64>>,
    gauges: HashMap<String, Gauge<f64>>,
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

    fn gauge(&mut self, meter: &Meter, name: &str, unit: &str) -> Gauge<f64> {
        if let Some(g) = self.gauges.get(name) {
            return g.clone();
        }
        let g = meter
            .f64_gauge(name.to_string())
            .with_unit(Unit::new(unit.to_string()))
            .init();
        self.gauges.insert(name.to_string(), g.clone());
        g
    }
}

pub(super) fn export_metric(runtime: &OtelRuntime, metric: &PerfMetric, cfg: &PerfTelemetryConfig) {
    let is_first = !FIRST_REMOTE_EXPORT.swap(true, Ordering::Relaxed);
    if is_first {
        tracing::info!(metric = %metric.name, "exporting perf metric via OTLP");
    }
    let labels = filter_labels(&metric.labels, cfg);
    let attrs = labels_to_kvs(&labels);
    let mut registry = lock_or_recover(&runtime.registry, "perf registry");
    match metric.kind {
        PerfMetricKind::Histogram => {
            let hist = registry.histogram(&runtime.meter, &metric.name, &metric.unit);
            hist.record(metric.value, &attrs);
        }
        PerfMetricKind::Counter => {
            let counter = registry.counter(&runtime.meter, &metric.name, &metric.unit);
            counter.add(metric.value as u64, &attrs);
        }
        PerfMetricKind::Gauge => {
            let gauge = registry.gauge(&runtime.meter, &metric.name, &metric.unit);
            gauge.record(metric.value, &attrs);
        }
    }
    drop(registry);

    if is_first {
        let meter_provider = runtime._meter_provider.clone();
        tokio::spawn(async move {
            let flush = tokio::task::spawn_blocking(move || meter_provider.force_flush());
            match timeout(Duration::from_secs(5), flush).await {
                Ok(Ok(Ok(()))) => tracing::info!("forced flush perf metrics"),
                Ok(Ok(Err(err))) => {
                    tracing::warn!("failed to force flush perf metrics: {err}")
                }
                Ok(Err(err)) => tracing::warn!("failed to force flush perf metrics task: {err}"),
                Err(_) => tracing::warn!("force flush perf metrics timed out"),
            }
        });
    }
}

pub(super) fn build_otel(cfg: &PerfTelemetryConfig) -> Option<Arc<OtelRuntime>> {
    let endpoint = cfg.otlp_endpoint.as_ref()?;
    let trace_endpoint = otlp_endpoint_for_signal(endpoint, "traces");
    let metric_endpoint = otlp_endpoint_for_signal(endpoint, "metrics");
    tracing::info!(
        otlp_traces_endpoint = %trace_endpoint,
        otlp_metrics_endpoint = %metric_endpoint,
        "perf telemetry OTLP exporter configured"
    );
    let resource = Resource::new(vec![
        KeyValue::new("service.name", "ctx-daemon"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
        KeyValue::new("os.type", std::env::consts::OS.to_string()),
        KeyValue::new("host.arch", std::env::consts::ARCH.to_string()),
    ]);

    global::set_text_map_propagator(TraceContextPropagator::new());

    let trace_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(trace_endpoint.clone())
        .with_headers(cfg.otlp_headers.clone());

    let trace_exporter = match trace_exporter.build_span_exporter() {
        Ok(exporter) => exporter,
        Err(err) => {
            tracing::warn!("failed to initialize OTLP trace exporter: {err}");
            return None;
        }
    };

    let trace_config = TraceConfig::default()
        .with_resource(resource.clone())
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            cfg.traces_sample_rate,
        ))));
    let trace_processor = BatchSpanProcessor::builder(trace_exporter, runtime::Tokio).build();
    let trace_provider = SdkTracerProvider::builder()
        .with_span_processor(trace_processor)
        .with_config(trace_config)
        .build();
    let tracer = trace_provider
        .tracer_builder("ctx-daemon")
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();

    let slow_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(trace_endpoint)
        .with_headers(cfg.otlp_headers.clone());
    let slow_exporter = match slow_exporter.build_span_exporter() {
        Ok(exporter) => exporter,
        Err(err) => {
            tracing::warn!("failed to initialize OTLP slow trace exporter: {err}");
            return None;
        }
    };
    let slow_config = TraceConfig::default()
        .with_resource(resource.clone())
        .with_sampler(Sampler::AlwaysOn);
    let slow_processor = BatchSpanProcessor::builder(slow_exporter, runtime::Tokio).build();
    let slow_trace_provider = SdkTracerProvider::builder()
        .with_span_processor(slow_processor)
        .with_config(slow_config)
        .build();
    let slow_tracer = slow_trace_provider
        .tracer_builder("ctx-daemon-slow")
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();

    let metric_exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(metric_endpoint)
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
        _trace_provider: trace_provider,
        _slow_trace_provider: slow_trace_provider,
        meter,
        _meter_provider: meter_provider,
        registry: Mutex::new(MetricRegistry::default()),
    }))
}

fn otlp_endpoint_for_signal(base: &str, signal: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if let Some(prefix) = trimmed.strip_suffix("/v1") {
        return format!("{prefix}/v1/{signal}");
    }
    if let Some((prefix, _)) = trimmed.rsplit_once("/v1/") {
        return format!("{prefix}/v1/{signal}");
    }
    format!("{trimmed}/v1/{signal}")
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

pub(super) fn extract_trace_context(headers: &http::HeaderMap) -> OtelContext {
    let extractor = HeaderExtractor { headers };
    global::get_text_map_propagator(|prop| prop.extract(&extractor))
}

pub(super) fn trace_id_to_string(id: TraceId) -> String {
    format!("{id:032x}")
}

pub(super) fn span_id_to_string(id: opentelemetry::trace::SpanId) -> String {
    format!("{id:016x}")
}

struct HeaderExtractor<'a> {
    headers: &'a http::HeaderMap,
}

impl<'a> Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.headers.keys().map(|k| k.as_str()).collect()
    }
}
