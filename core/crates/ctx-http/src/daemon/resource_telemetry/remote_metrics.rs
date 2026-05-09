use std::collections::HashMap;

use serde::Serialize;

use ctx_avf_linux_runtime::SubstrateLifecycleRecord;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use ctx_resource_utilization::{ResourceProcesses, SystemSnapshot};

pub(super) fn export_remote_metrics(
    perf: &PerfTelemetry,
    system: &SystemSnapshot,
    processes: &ResourceProcesses,
    provider_sessions: &HashMap<String, u64>,
    shared_substrate_lifecycle: Option<&SubstrateLifecycleRecord>,
) {
    let labels = HashMap::new();
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.cpu_pct".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "percent".to_string(),
        value: system.cpu_pct as f64,
        labels: labels.clone(),
    });
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.mem_used_bytes".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "bytes".to_string(),
        value: system.memory_used_bytes as f64,
        labels: labels.clone(),
    });
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.swap_used_bytes".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "bytes".to_string(),
        value: system.swap_used_bytes as f64,
        labels: labels.clone(),
    });

    if let Some(daemon) = processes.daemon.as_ref() {
        perf.export_remote_metric(PerfMetric {
            name: "ctx.daemon.cpu_pct".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "percent".to_string(),
            value: daemon.cpu_pct as f64,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.daemon.mem_bytes".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bytes".to_string(),
            value: daemon.memory_bytes as f64,
            labels: labels.clone(),
        });
    }

    let mut provider_agg: HashMap<String, ProviderAggregate> = HashMap::new();
    for proc in processes.providers.iter() {
        let label = proc.label.clone();
        let entry = provider_agg.entry(label).or_default();
        entry.cpu_pct += proc.cpu_pct as f64;
        entry.mem_bytes += proc.memory_bytes;
        entry.process_count += 1;
    }

    for (provider_id, agg) in provider_agg.iter() {
        let mut labels = HashMap::new();
        labels.insert("provider_id".to_string(), provider_id.clone());
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.cpu_pct".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "percent".to_string(),
            value: agg.cpu_pct,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.mem_bytes".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bytes".to_string(),
            value: agg.mem_bytes as f64,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.process_count".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "count".to_string(),
            value: agg.process_count as f64,
            labels: labels.clone(),
        });
    }

    for (provider_id, count) in provider_sessions.iter() {
        let mut labels = HashMap::new();
        labels.insert("provider_id".to_string(), provider_id.clone());
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.session_count".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "count".to_string(),
            value: *count as f64,
            labels,
        });
    }

    if let Some(record) = shared_substrate_lifecycle {
        let mut base_labels = HashMap::new();
        if let Some(substrate) = serde_label(&record.substrate) {
            base_labels.insert("substrate_kind".to_string(), substrate);
        }

        perf.export_remote_metric(PerfMetric {
            name: "ctx.substrate.simulated".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bool".to_string(),
            value: if record.simulated { 1.0 } else { 0.0 },
            labels: base_labels.clone(),
        });

        if let Some(startup_selection) = record.startup_selection.as_ref().and_then(serde_label) {
            let mut labels = base_labels.clone();
            labels.insert("startup_selection".to_string(), startup_selection);
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.startup_selection".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }

        if let Some(startup_outcome) = record.startup_outcome.as_ref().and_then(serde_label) {
            let mut labels = base_labels.clone();
            labels.insert("startup_outcome".to_string(), startup_outcome);
            if let Some(startup_reason) = record.startup_reason.as_ref().and_then(serde_label) {
                labels.insert("startup_reason".to_string(), startup_reason);
            }
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.startup_outcome".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }

        if let Some(shutdown_outcome) = record.shutdown_outcome.as_ref().and_then(serde_label) {
            let mut labels = base_labels;
            labels.insert("shutdown_outcome".to_string(), shutdown_outcome);
            if let Some(shutdown_reason) = record.shutdown_reason.as_ref().and_then(serde_label) {
                labels.insert("shutdown_reason".to_string(), shutdown_reason);
            }
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.shutdown_outcome".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }
    }
}

#[derive(Default)]
struct ProviderAggregate {
    cpu_pct: f64,
    mem_bytes: u64,
    process_count: u64,
}

fn serde_label<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()?
        .as_str()
        .map(ToString::to_string)
}
