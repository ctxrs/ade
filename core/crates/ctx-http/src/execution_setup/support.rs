use super::*;

impl ExecutionSetupCoordinator {
    pub(super) fn emit_phase(&self, job: &Arc<LaunchJob>, phase: HarnessSetupPhase, message: &str) {
        if job.is_terminal() {
            return;
        }
        let update = job.transition_phase(phase, message);
        if let Some(completed) = update.completed_phase {
            self.record_phase_metric(completed.phase, completed.elapsed_ms, "running");
        }
        if update.snapshot_changed {
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchSnapshot {
                snapshot: update.snapshot,
            });
        }
        if let Some(line) = update.line {
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchLog {
                job_id: job.job_id.clone(),
                line,
            });
        }
    }

    pub(super) fn emit_progress(&self, job: &Arc<LaunchJob>, progress: HarnessSetupProgressUpdate) {
        if job.is_terminal() {
            return;
        }
        let update = job.set_progress(progress);
        if update.snapshot_changed {
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchSnapshot {
                snapshot: update.snapshot,
            });
        }
    }

    pub(super) fn emit_log(
        &self,
        job: &Arc<LaunchJob>,
        phase: HarnessSetupPhase,
        level: HarnessSetupLogLevel,
        message: &str,
    ) {
        if job.is_terminal() {
            return;
        }
        let update = job.push_log(phase, level, message);
        if let Some(line) = update.line {
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchLog {
                job_id: job.job_id.clone(),
                line,
            });
        }
    }

    pub(super) fn record_phase_metric(
        &self,
        phase: HarnessSetupPhase,
        elapsed_ms: u64,
        result: &str,
    ) {
        let mut labels = HashMap::new();
        labels.insert("phase".to_string(), phase_label(phase).to_string());
        labels.insert("result".to_string(), result.to_string());
        let metric = PerfMetric {
            name: "execution.launch.phase_duration_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: elapsed_ms as f64,
            labels,
        };
        let perf = self.perf_telemetry.clone();
        tokio::spawn(async move {
            perf.record_metric(metric, None, None, None).await;
        });
    }

    pub(super) fn record_launch_metric(&self, elapsed_ms: u64, result: &str) {
        let mut labels = HashMap::new();
        labels.insert("result".to_string(), result.to_string());
        let metric = PerfMetric {
            name: "execution.launch.total_duration_ms".to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: elapsed_ms as f64,
            labels,
        };
        let perf = self.perf_telemetry.clone();
        tokio::spawn(async move {
            perf.record_metric(metric, None, None, None).await;
        });
    }
}
