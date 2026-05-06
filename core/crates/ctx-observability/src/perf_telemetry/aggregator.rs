use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{PerfMetric, PerfMetricKind, PerfMetricSummary, PerfTelemetryStats};

#[derive(Default)]
pub(super) struct PerfAggregator {
    metrics: HashMap<MetricKey, MetricWindow>,
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
    pub(super) fn record(&mut self, metric: &PerfMetric, run_id: Option<&str>) {
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

    pub(super) fn summary(
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

fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values.get(idx).copied()
}

impl PerfAggregator {
    pub(super) fn stats(&self) -> PerfTelemetryStats {
        let mut total_samples = 0;
        let mut max_samples = 0;
        for window in self.metrics.values() {
            let count = window.samples.len();
            total_samples += count;
            if count > max_samples {
                max_samples = count;
            }
        }
        PerfTelemetryStats {
            metric_keys: self.metrics.len(),
            total_samples,
            max_samples,
        }
    }
}
