use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_store::Store;

use crate::harness_runtime::{
    self, HarnessRuntimeManager, HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase,
};
use crate::ops_events::{OpsEvent, OpsEvents};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::settings::{ExecutionMode, ExecutionSettings};

const JOB_LOG_CAP: usize = 400;
const JOB_HISTORY_CAP: usize = 128;
const LAUNCH_EVENT_CHANNEL_CAP: usize = 256;

fn lock_or_recover<'a, T>(mutex: &'a StdMutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSetupJobKind {
    StartupPrewarm,
    WorkspaceLaunch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePrewarmScope {
    #[default]
    Runtime,
    Builder,
    All,
}

impl RuntimePrewarmScope {
    fn includes_runtime(self) -> bool {
        matches!(self, Self::Runtime | Self::All)
    }

    fn includes_builder(self) -> bool {
        matches!(self, Self::Builder | Self::All)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLaunchState {
    Running,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLaunchLogLine {
    pub seq: u64,
    pub ts: String,
    pub phase: HarnessSetupPhase,
    pub level: HarnessSetupLogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLaunchPhaseStatus {
    pub phase: HarnessSetupPhase,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLaunchSnapshot {
    pub job_id: String,
    pub workspace_id: String,
    pub kind: ExecutionSetupJobKind,
    pub state: ExecutionLaunchState,
    pub created_at: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<HarnessSetupPhase>,
    pub phases: Vec<ExecutionLaunchPhaseStatus>,
    pub logs: Vec<ExecutionLaunchLogLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionLaunchStreamEvent {
    LaunchSnapshot {
        snapshot: ExecutionLaunchSnapshot,
    },
    LaunchLog {
        job_id: String,
        line: ExecutionLaunchLogLine,
    },
    LaunchComplete {
        snapshot: ExecutionLaunchSnapshot,
    },
    LaunchError {
        snapshot: ExecutionLaunchSnapshot,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartupPrewarmState {
    #[default]
    Idle,
    Running,
    Ready,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupPrewarmSnapshot {
    pub state: StartupPrewarmState,
    pub target_image: String,
    pub needs_prewarm: bool,
    pub machine_ready: bool,
    pub image_present: bool,
    pub image_ref_changed: bool,
    pub bundled_image_digest_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct CoordinatorState {
    launch_jobs: HashMap<String, Arc<LaunchJob>>,
    launch_history: VecDeque<String>,
    running_launch_by_workspace: HashMap<WorkspaceId, String>,
    startup: StartupPrewarmSnapshot,
}

#[derive(Debug)]
struct LaunchPhaseRecord {
    phase: HarnessSetupPhase,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    elapsed_ms: Option<u64>,
}

#[derive(Debug)]
struct LaunchJobInner {
    kind: ExecutionSetupJobKind,
    state: ExecutionLaunchState,
    created_at: DateTime<Utc>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    current_phase: Option<HarnessSetupPhase>,
    phases: Vec<LaunchPhaseRecord>,
    logs: VecDeque<ExecutionLaunchLogLine>,
    next_seq: u64,
    error: Option<String>,
}

impl LaunchJobInner {
    fn new(kind: ExecutionSetupJobKind) -> Self {
        let now = Utc::now();
        Self {
            kind,
            state: ExecutionLaunchState::Running,
            created_at: now,
            started_at: now,
            finished_at: None,
            current_phase: None,
            phases: Vec::new(),
            logs: VecDeque::new(),
            next_seq: 0,
            error: None,
        }
    }

    fn push_log(
        &mut self,
        phase: HarnessSetupPhase,
        level: HarnessSetupLogLevel,
        message: &str,
        now: DateTime<Utc>,
    ) -> ExecutionLaunchLogLine {
        self.next_seq += 1;
        let line = ExecutionLaunchLogLine {
            seq: self.next_seq,
            ts: format_ts(now),
            phase,
            level,
            message: message.to_string(),
        };
        self.logs.push_back(line.clone());
        while self.logs.len() > JOB_LOG_CAP {
            self.logs.pop_front();
        }
        line
    }

    fn close_current_phase(&mut self, now: DateTime<Utc>) -> Option<CompletedPhase> {
        let phase = self.current_phase?;
        let Some(last) = self.phases.last_mut() else {
            self.current_phase = None;
            return None;
        };
        if last.phase != phase || last.finished_at.is_some() {
            self.current_phase = None;
            return None;
        }
        last.finished_at = Some(now);
        let elapsed = now.signed_duration_since(last.started_at);
        let elapsed_ms = elapsed.num_milliseconds().max(0) as u64;
        last.elapsed_ms = Some(elapsed_ms);
        self.current_phase = None;
        Some(CompletedPhase { phase, elapsed_ms })
    }

    fn snapshot(&self, job_id: &str, workspace_id: WorkspaceId) -> ExecutionLaunchSnapshot {
        ExecutionLaunchSnapshot {
            job_id: job_id.to_string(),
            workspace_id: workspace_id.0.to_string(),
            kind: self.kind,
            state: self.state,
            created_at: format_ts(self.created_at),
            started_at: format_ts(self.started_at),
            finished_at: self.finished_at.map(format_ts),
            current_phase: self.current_phase,
            phases: self
                .phases
                .iter()
                .map(|phase| ExecutionLaunchPhaseStatus {
                    phase: phase.phase,
                    started_at: format_ts(phase.started_at),
                    finished_at: phase.finished_at.map(format_ts),
                    elapsed_ms: phase.elapsed_ms,
                })
                .collect(),
            logs: self.logs.iter().cloned().collect(),
            error: self.error.clone(),
        }
    }
}

#[derive(Debug)]
struct LaunchJob {
    job_id: String,
    workspace_id: WorkspaceId,
    tx: broadcast::Sender<ExecutionLaunchStreamEvent>,
    inner: StdMutex<LaunchJobInner>,
}

impl LaunchJob {
    fn new(job_id: String, workspace_id: WorkspaceId) -> Self {
        let (tx, _) = broadcast::channel(LAUNCH_EVENT_CHANNEL_CAP);
        Self {
            job_id,
            workspace_id,
            tx,
            inner: StdMutex::new(LaunchJobInner::new(ExecutionSetupJobKind::WorkspaceLaunch)),
        }
    }

    fn snapshot(&self) -> ExecutionLaunchSnapshot {
        let inner = lock_or_recover(&self.inner, "launch job");
        inner.snapshot(&self.job_id, self.workspace_id)
    }

    fn current_phase(&self) -> Option<HarnessSetupPhase> {
        let inner = lock_or_recover(&self.inner, "launch job");
        inner.current_phase
    }

    fn transition_phase(&self, phase: HarnessSetupPhase, message: &str) -> LaunchMutation {
        let now = Utc::now();
        let mut inner = lock_or_recover(&self.inner, "launch job");
        let mut completed_phase = None;
        let mut phase_changed = false;
        if inner.current_phase != Some(phase) {
            completed_phase = inner.close_current_phase(now);
            inner.current_phase = Some(phase);
            inner.phases.push(LaunchPhaseRecord {
                phase,
                started_at: now,
                finished_at: None,
                elapsed_ms: None,
            });
            phase_changed = true;
        }
        let line = inner.push_log(phase, HarnessSetupLogLevel::Info, message, now);
        let snapshot = inner.snapshot(&self.job_id, self.workspace_id);
        LaunchMutation {
            line: Some(line),
            snapshot,
            phase_changed,
            completed_phase,
        }
    }

    fn push_log(
        &self,
        phase: HarnessSetupPhase,
        level: HarnessSetupLogLevel,
        message: &str,
    ) -> LaunchMutation {
        let now = Utc::now();
        let mut inner = lock_or_recover(&self.inner, "launch job");
        let line = inner.push_log(phase, level, message, now);
        let snapshot = inner.snapshot(&self.job_id, self.workspace_id);
        LaunchMutation {
            line: Some(line),
            snapshot,
            phase_changed: false,
            completed_phase: None,
        }
    }

    fn mark_terminal(
        &self,
        state: ExecutionLaunchState,
        error: Option<String>,
    ) -> LaunchTerminalMutation {
        let now = Utc::now();
        let mut inner = lock_or_recover(&self.inner, "launch job");
        let completed_phase = inner.close_current_phase(now);
        inner.state = state;
        inner.finished_at = Some(now);
        inner.error = error;
        let snapshot = inner.snapshot(&self.job_id, self.workspace_id);
        LaunchTerminalMutation {
            snapshot,
            completed_phase,
        }
    }
}

#[derive(Debug)]
struct CompletedPhase {
    phase: HarnessSetupPhase,
    elapsed_ms: u64,
}

#[derive(Debug)]
struct LaunchMutation {
    line: Option<ExecutionLaunchLogLine>,
    snapshot: ExecutionLaunchSnapshot,
    phase_changed: bool,
    completed_phase: Option<CompletedPhase>,
}

#[derive(Debug)]
struct LaunchTerminalMutation {
    snapshot: ExecutionLaunchSnapshot,
    completed_phase: Option<CompletedPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartupPrewarmMetadata {
    image_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundled_image_fingerprint: Option<String>,
    ready_at: String,
}

#[derive(Debug)]
struct PrewarmGate {
    machine_ready: bool,
    image_present: bool,
    image_ref_changed: bool,
    bundled_image_digest_changed: bool,
    needs_prewarm: bool,
    bundled_image_fingerprint: Option<String>,
}

pub struct ExecutionSetupCoordinator {
    data_root: PathBuf,
    harness: Arc<HarnessRuntimeManager>,
    perf_telemetry: PerfTelemetry,
    ops_events: OpsEvents,
    inner: Mutex<CoordinatorState>,
}

impl ExecutionSetupCoordinator {
    pub fn new(
        data_root: PathBuf,
        harness: Arc<HarnessRuntimeManager>,
        perf_telemetry: PerfTelemetry,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            harness,
            perf_telemetry,
            ops_events,
            inner: Mutex::new(CoordinatorState::default()),
        }
    }

    pub fn spawn_startup_prewarm(self: &Arc<Self>) {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator.run_startup_prewarm().await;
        });
    }

    pub async fn start_workspace_launch(
        self: &Arc<Self>,
        workspace: Workspace,
        settings: ExecutionSettings,
        daemon_url: String,
    ) -> ExecutionLaunchSnapshot {
        let (job, snapshot) = {
            let mut inner = self.inner.lock().await;

            if let Some(existing_job_id) = inner
                .running_launch_by_workspace
                .get(&workspace.id)
                .cloned()
            {
                if let Some(existing) = inner.launch_jobs.get(&existing_job_id) {
                    return existing.snapshot();
                }
                // Stale pointer: drop it so a new launch can start cleanly.
                inner.running_launch_by_workspace.remove(&workspace.id);
            }

            let job_id = uuid::Uuid::new_v4().to_string();
            let job = Arc::new(LaunchJob::new(job_id.clone(), workspace.id));
            let snapshot = job.snapshot();

            inner
                .running_launch_by_workspace
                .insert(workspace.id, job_id.clone());
            inner.launch_jobs.insert(job_id, Arc::clone(&job));
            inner.launch_history.push_back(job.job_id.clone());
            while inner.launch_history.len() > JOB_HISTORY_CAP {
                let Some(old_id) = inner.launch_history.pop_front() else {
                    break;
                };
                let still_running = inner
                    .running_launch_by_workspace
                    .values()
                    .any(|active_id| active_id == &old_id);
                if !still_running {
                    inner.launch_jobs.remove(&old_id);
                }
            }

            (job, snapshot)
        };

        let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchSnapshot {
            snapshot: snapshot.clone(),
        });

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator
                .run_workspace_launch(job, workspace, settings, daemon_url)
                .await;
        });

        snapshot
    }

    pub async fn start_runtime_prewarm(
        self: &Arc<Self>,
        settings: ExecutionSettings,
        scope: RuntimePrewarmScope,
    ) -> ExecutionLaunchSnapshot {
        let workspace_id = WorkspaceId(uuid::Uuid::nil());
        let (job, snapshot) = {
            let mut inner = self.inner.lock().await;
            let job_id = uuid::Uuid::new_v4().to_string();
            let job = Arc::new(LaunchJob::new(job_id.clone(), workspace_id));
            {
                let mut job_inner = lock_or_recover(&job.inner, "launch_job_inner");
                job_inner.kind = ExecutionSetupJobKind::StartupPrewarm;
            }
            let snapshot = job.snapshot();
            inner.launch_jobs.insert(job_id, Arc::clone(&job));
            inner.launch_history.push_back(job.job_id.clone());
            while inner.launch_history.len() > JOB_HISTORY_CAP {
                let Some(old_id) = inner.launch_history.pop_front() else {
                    break;
                };
                let still_running = inner
                    .running_launch_by_workspace
                    .values()
                    .any(|active_id| active_id == &old_id);
                if !still_running {
                    inner.launch_jobs.remove(&old_id);
                }
            }
            (job, snapshot)
        };

        let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchSnapshot {
            snapshot: snapshot.clone(),
        });

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator.run_runtime_prewarm(job, settings, scope).await;
        });

        snapshot
    }

    pub async fn launch_status(&self, job_id: &str) -> Option<ExecutionLaunchSnapshot> {
        let job = {
            let inner = self.inner.lock().await;
            inner.launch_jobs.get(job_id).cloned()
        }?;
        Some(job.snapshot())
    }

    pub async fn subscribe_launch(
        &self,
        job_id: &str,
    ) -> Option<(
        ExecutionLaunchSnapshot,
        broadcast::Receiver<ExecutionLaunchStreamEvent>,
    )> {
        let job = {
            let inner = self.inner.lock().await;
            inner.launch_jobs.get(job_id).cloned()
        }?;
        let rx = job.tx.subscribe();
        let snapshot = job.snapshot();
        Some((snapshot, rx))
    }

    pub async fn startup_status(&self) -> StartupPrewarmSnapshot {
        let inner = self.inner.lock().await;
        inner.startup.clone()
    }

    async fn run_workspace_launch(
        self: Arc<Self>,
        job: Arc<LaunchJob>,
        workspace: Workspace,
        settings: ExecutionSettings,
        daemon_url: String,
    ) {
        let launch_started = std::time::Instant::now();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let mut attempt = 0usize;
        let run_result = loop {
            attempt += 1;
            let attempt_result = if is_host_mode {
                self.emit_phase(
                    &job,
                    HarnessSetupPhase::Ready,
                    "host execution mode selected; container launch skipped",
                );
                Ok(())
            } else {
                self.harness
                    .ensure_workspace_container_with_observer(
                        &workspace,
                        &settings,
                        &daemon_url,
                        Some(&observer),
                    )
                    .await
                    .context("container runtime failed")
            };
            match attempt_result {
                Ok(()) => break Ok(()),
                Err(_err) if should_retry_machine_start_failure(&job, attempt, is_host_mode) => {
                    self.emit_log(
                        &job,
                        HarnessSetupPhase::MachineStartOrInit,
                        HarnessSetupLogLevel::Warn,
                        "container runtime machine startup failed; retrying once",
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
                Err(err) => break Err(err),
            }
        };

        match run_result {
            Ok(()) => {
                if !matches!(settings.mode, ExecutionMode::Host) {
                    self.emit_phase(&job, HarnessSetupPhase::Ready, "workspace runtime is ready");
                }
                let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            Err(err) => {
                let message = format_error_chain(&err);
                let phase = job
                    .current_phase()
                    .unwrap_or(HarnessSetupPhase::ContainerStartOrCreate);
                self.emit_log(&job, phase, HarnessSetupLogLevel::Error, &message);
                let terminal =
                    job.mark_terminal(ExecutionLaunchState::Error, Some(message.clone()));
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "error");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchError {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "error");

                let mut event = OpsEvent::new("error", "execution.launch_error");
                event.meta = Some(json!({
                    "job_id": terminal.snapshot.job_id,
                    "workspace_id": terminal.snapshot.workspace_id,
                    "phase": terminal.snapshot.current_phase,
                    "error": message,
                }));
                self.ops_events.emit(event);
            }
        }

        self.clear_running_launch(workspace.id, &job.job_id).await;
    }

    async fn run_runtime_prewarm(
        self: Arc<Self>,
        job: Arc<LaunchJob>,
        settings: ExecutionSettings,
        scope: RuntimePrewarmScope,
    ) {
        let launch_started = std::time::Instant::now();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let mut attempt = 0usize;
        let run_result = loop {
            attempt += 1;
            let attempt_result = if is_host_mode {
                self.emit_phase(
                    &job,
                    HarnessSetupPhase::Ready,
                    "host execution mode selected; runtime prewarm skipped",
                );
                Ok(())
            } else if !harness_runtime::container_runtime_available(&self.data_root) {
                Err(anyhow::anyhow!("container runtime unavailable"))
            } else {
                let prewarm_result = async {
                    if scope.includes_runtime() {
                        let image = harness_runtime::resolve_container_image(&settings.container);
                        harness_runtime::prefetch_container_image_with_observer(
                            &self.data_root,
                            &image,
                            Some(&observer),
                        )
                        .await
                        .context("container runtime failed")?;
                    }
                    if scope.includes_builder() {
                        self.emit_phase(
                            &job,
                            HarnessSetupPhase::ImageLoad,
                            "warming container builder",
                        );
                        crate::container_builder::ensure_builder_ready(&self.data_root)
                            .await
                            .context("container builder warmup failed")?;
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                prewarm_result
            };
            match attempt_result {
                Ok(()) => break Ok(()),
                Err(_err) if should_retry_machine_start_failure(&job, attempt, is_host_mode) => {
                    self.emit_log(
                        &job,
                        HarnessSetupPhase::MachineStartOrInit,
                        HarnessSetupLogLevel::Warn,
                        "container runtime machine startup failed; retrying once",
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
                Err(err) => break Err(err),
            }
        };

        match run_result {
            Ok(()) => {
                if !matches!(settings.mode, ExecutionMode::Host) {
                    self.emit_phase(&job, HarnessSetupPhase::Ready, "container runtime is ready");
                }
                let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            Err(err) => {
                let message = format_error_chain(&err);
                let phase = job.current_phase().unwrap_or(HarnessSetupPhase::ImageLoad);
                self.emit_log(&job, phase, HarnessSetupLogLevel::Error, &message);
                let terminal =
                    job.mark_terminal(ExecutionLaunchState::Error, Some(message.clone()));
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "error");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchError {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "error");

                let mut event = OpsEvent::new("error", "execution.runtime_prewarm_error");
                event.meta = Some(json!({
                    "job_id": terminal.snapshot.job_id,
                    "phase": terminal.snapshot.current_phase,
                    "error": message,
                }));
                self.ops_events.emit(event);
            }
        }
    }

    fn emit_phase(&self, job: &Arc<LaunchJob>, phase: HarnessSetupPhase, message: &str) {
        let update = job.transition_phase(phase, message);
        if let Some(completed) = update.completed_phase {
            self.record_phase_metric(completed.phase, completed.elapsed_ms, "running");
        }
        if update.phase_changed {
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

    fn emit_log(
        &self,
        job: &Arc<LaunchJob>,
        phase: HarnessSetupPhase,
        level: HarnessSetupLogLevel,
        message: &str,
    ) {
        let update = job.push_log(phase, level, message);
        if let Some(line) = update.line {
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchLog {
                job_id: job.job_id.clone(),
                line,
            });
        }
    }

    fn record_phase_metric(&self, phase: HarnessSetupPhase, elapsed_ms: u64, result: &str) {
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

    fn record_launch_metric(&self, elapsed_ms: u64, result: &str) {
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

    async fn clear_running_launch(&self, workspace_id: WorkspaceId, job_id: &str) {
        let mut inner = self.inner.lock().await;
        if inner
            .running_launch_by_workspace
            .get(&workspace_id)
            .map(String::as_str)
            == Some(job_id)
        {
            inner.running_launch_by_workspace.remove(&workspace_id);
        }
    }

    async fn run_startup_prewarm(&self) {
        let attempted_at = format_ts(Utc::now());
        let db_path = self.data_root.join("db").join("db.sqlite");
        let settings = match Store::open_sqlite(&db_path, None).await {
            Ok(store) => {
                let loaded = crate::settings::load_settings(&store).await;
                store.close().await;
                match loaded {
                    Ok(settings) => settings,
                    Err(err) => {
                        let message = format!("failed to load execution settings: {err:#}");
                        let snapshot = StartupPrewarmSnapshot {
                            state: StartupPrewarmState::Error,
                            target_image: String::new(),
                            needs_prewarm: false,
                            machine_ready: false,
                            image_present: false,
                            image_ref_changed: false,
                            bundled_image_digest_changed: false,
                            last_attempt_at: Some(attempted_at.clone()),
                            last_success_at: None,
                            error: Some(message.clone()),
                        };
                        self.set_startup_snapshot(snapshot).await;
                        let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                        event.meta = Some(json!({"error": message}));
                        self.ops_events.emit(event);
                        return;
                    }
                }
            }
            Err(err) => {
                let message = format!("failed to open global settings store: {err:#}");
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: String::new(),
                    needs_prewarm: false,
                    machine_ready: false,
                    image_present: false,
                    image_ref_changed: false,
                    bundled_image_digest_changed: false,
                    last_attempt_at: Some(attempted_at.clone()),
                    last_success_at: None,
                    error: Some(message.clone()),
                };
                self.set_startup_snapshot(snapshot).await;
                let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                event.meta = Some(json!({"error": message}));
                self.ops_events.emit(event);
                return;
            }
        };
        let exec = settings.execution.unwrap_or_default();
        let image = harness_runtime::resolve_container_image(&exec.container);

        if !harness_runtime::container_runtime_available(&self.data_root) {
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Skipped,
                target_image: image,
                needs_prewarm: false,
                machine_ready: false,
                image_present: false,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at),
                last_success_at: None,
                error: Some("container runtime unavailable".to_string()),
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        {
            let mut inner = self.inner.lock().await;
            inner.startup = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Running,
                target_image: image.clone(),
                needs_prewarm: false,
                machine_ready: false,
                image_present: false,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at.clone()),
                last_success_at: inner.startup.last_success_at.clone(),
                error: None,
            };
        }

        let gate = match self.compute_prewarm_gate(&image).await {
            Ok(gate) => gate,
            Err(err) => {
                let message = format_error_chain(&err);
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: image.clone(),
                    needs_prewarm: true,
                    machine_ready: false,
                    image_present: false,
                    image_ref_changed: false,
                    bundled_image_digest_changed: false,
                    last_attempt_at: Some(attempted_at),
                    last_success_at: None,
                    error: Some(message.clone()),
                };
                self.set_startup_snapshot(snapshot).await;
                let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                event.meta = Some(json!({"error": message}));
                self.ops_events.emit(event);
                return;
            }
        };

        if !gate.needs_prewarm {
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Ready,
                target_image: image,
                needs_prewarm: false,
                machine_ready: gate.machine_ready,
                image_present: gate.image_present,
                image_ref_changed: gate.image_ref_changed,
                bundled_image_digest_changed: gate.bundled_image_digest_changed,
                last_attempt_at: Some(attempted_at),
                last_success_at: Some(format_ts(Utc::now())),
                error: None,
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        match harness_runtime::prefetch_container_image(&self.data_root, &image).await {
            Ok(()) => {
                let metadata = StartupPrewarmMetadata {
                    image_ref: image.clone(),
                    bundled_image_fingerprint: gate.bundled_image_fingerprint,
                    ready_at: format_ts(Utc::now()),
                };
                let _ = write_prewarm_metadata(&self.data_root, &metadata).await;
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Ready,
                    target_image: image,
                    needs_prewarm: true,
                    machine_ready: true,
                    image_present: true,
                    image_ref_changed: gate.image_ref_changed,
                    bundled_image_digest_changed: gate.bundled_image_digest_changed,
                    last_attempt_at: Some(attempted_at),
                    last_success_at: Some(metadata.ready_at),
                    error: None,
                };
                self.set_startup_snapshot(snapshot).await;
            }
            Err(err) => {
                let message = format_error_chain(&err);
                tracing::warn!("startup prewarm failed: {message}");
                let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                event.meta = Some(json!({"image": image, "error": message}));
                self.ops_events.emit(event);

                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: image,
                    needs_prewarm: true,
                    machine_ready: gate.machine_ready,
                    image_present: gate.image_present,
                    image_ref_changed: gate.image_ref_changed,
                    bundled_image_digest_changed: gate.bundled_image_digest_changed,
                    last_attempt_at: Some(attempted_at),
                    last_success_at: None,
                    error: Some(err.to_string()),
                };
                self.set_startup_snapshot(snapshot).await;
            }
        }
    }

    async fn compute_prewarm_gate(&self, image: &str) -> Result<PrewarmGate> {
        let metadata = read_prewarm_metadata(&self.data_root).await?;
        let machine_ready = harness_runtime::podman_engine_ready(&self.data_root).await?;
        let image_present = if machine_ready {
            harness_runtime::container_image_present(&self.data_root, image).await?
        } else {
            false
        };
        let bundled_image_fingerprint = bundled_image_fingerprint(image).await?;

        let image_ref_changed = metadata
            .as_ref()
            .map(|meta| meta.image_ref != image)
            .unwrap_or(false);
        let bundled_image_digest_changed = metadata
            .as_ref()
            .map(|meta| meta.bundled_image_fingerprint != bundled_image_fingerprint)
            .unwrap_or(false);

        let needs_prewarm = needs_prewarm(
            machine_ready,
            image_present,
            image_ref_changed,
            bundled_image_digest_changed,
        );

        Ok(PrewarmGate {
            machine_ready,
            image_present,
            image_ref_changed,
            bundled_image_digest_changed,
            needs_prewarm,
            bundled_image_fingerprint,
        })
    }

    async fn set_startup_snapshot(&self, snapshot: StartupPrewarmSnapshot) {
        let mut inner = self.inner.lock().await;
        inner.startup = snapshot;
    }
}

struct LaunchObserver {
    coordinator: Arc<ExecutionSetupCoordinator>,
    job: Arc<LaunchJob>,
}

impl HarnessSetupObserver for LaunchObserver {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str) {
        self.coordinator.emit_phase(&self.job, phase, message);
    }

    fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str) {
        self.coordinator.emit_log(&self.job, phase, level, message);
    }
}

fn phase_label(phase: HarnessSetupPhase) -> &'static str {
    match phase {
        HarnessSetupPhase::MachineCheck => "machine_check",
        HarnessSetupPhase::MachineStartOrInit => "machine_start_or_init",
        HarnessSetupPhase::ImageCheck => "image_check",
        HarnessSetupPhase::ImageLoad => "image_load",
        HarnessSetupPhase::ContainerCheck => "container_check",
        HarnessSetupPhase::ContainerStartOrCreate => "container_start_or_create",
        HarnessSetupPhase::RuntimeNetworkSetup => "runtime_network_setup",
        HarnessSetupPhase::Ready => "ready",
    }
}

fn should_retry_machine_start_failure(
    job: &Arc<LaunchJob>,
    attempt: usize,
    is_host_mode: bool,
) -> bool {
    !is_host_mode
        && attempt == 1
        && matches!(
            job.current_phase(),
            Some(HarnessSetupPhase::MachineStartOrInit)
        )
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in err.chain() {
        let raw = cause.to_string();
        let normalized = if raw.trim().is_empty() {
            "<empty error cause>".to_string()
        } else {
            raw.trim().to_string()
        };
        if parts.last() != Some(&normalized) {
            parts.push(normalized);
        }
    }
    if parts.is_empty() {
        return "unknown error".to_string();
    }
    parts.join(": ")
}

fn format_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn prewarm_metadata_path(data_root: &Path) -> PathBuf {
    data_root
        .join("execution")
        .join("startup_prewarm_status.json")
}

async fn read_prewarm_metadata(data_root: &Path) -> Result<Option<StartupPrewarmMetadata>> {
    let path = prewarm_metadata_path(data_root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let parsed = serde_json::from_str::<StartupPrewarmMetadata>(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(parsed))
}

async fn write_prewarm_metadata(data_root: &Path, metadata: &StartupPrewarmMetadata) -> Result<()> {
    let path = prewarm_metadata_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(metadata)?;
    tokio::fs::write(&path, raw)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

async fn bundled_image_fingerprint(image: &str) -> Result<Option<String>> {
    if !harness_runtime::is_default_container_image(image) {
        return Ok(None);
    }
    let Some(tar_path) = harness_runtime::bundled_default_container_image_tar() else {
        return Ok(None);
    };
    let metadata = tokio::fs::metadata(&tar_path)
        .await
        .with_context(|| format!("stat {}", tar_path.display()))?;
    let len = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    Ok(Some(format!("{len}:{modified}")))
}

fn needs_prewarm(
    machine_ready: bool,
    image_present: bool,
    image_ref_changed: bool,
    bundled_image_digest_changed: bool,
) -> bool {
    !machine_ready || !image_present || image_ref_changed || bundled_image_digest_changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use tokio::sync::Barrier;

    use crate::harness_runtime::HarnessRuntimeManager;
    use crate::ops_events::OpsEvents;
    use crate::perf_telemetry::PerfTelemetry;
    use crate::settings::{ExecutionMode, ExecutionSettings};

    fn test_workspace(id: WorkspaceId) -> Workspace {
        Workspace {
            id,
            name: "ws".to_string(),
            root_path: "/tmp/ws".to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        }
    }

    fn test_coordinator(data_root: PathBuf) -> Arc<ExecutionSetupCoordinator> {
        Arc::new(ExecutionSetupCoordinator::new(
            data_root.clone(),
            Arc::new(HarnessRuntimeManager::new(data_root.clone())),
            PerfTelemetry::new(data_root.clone()),
            OpsEvents::new(data_root),
        ))
    }

    #[test]
    fn needs_prewarm_gate_matches_truth_table() {
        assert!(needs_prewarm(false, false, false, false));
        assert!(needs_prewarm(true, false, false, false));
        assert!(needs_prewarm(true, true, true, false));
        assert!(needs_prewarm(true, true, false, true));
        assert!(!needs_prewarm(true, true, false, false));
    }

    #[test]
    fn launch_job_logs_are_bounded_and_ordered() {
        let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
        for i in 0..(JOB_LOG_CAP + 32) {
            let msg = format!("line {i}");
            let _ = job.push_log(
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Info,
                &msg,
            );
        }
        let snapshot = job.snapshot();
        assert_eq!(snapshot.logs.len(), JOB_LOG_CAP);
        let first = snapshot.logs.first().expect("missing first line");
        let last = snapshot.logs.last().expect("missing last line");
        assert!(first.seq < last.seq);
        let mut prev = first.seq;
        for line in snapshot.logs.iter().skip(1) {
            assert!(line.seq > prev);
            prev = line.seq;
        }
    }

    #[test]
    fn phase_transition_closes_previous_phase() {
        let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
        let _ = job.transition_phase(HarnessSetupPhase::MachineCheck, "check");
        let _ = job.transition_phase(HarnessSetupPhase::ImageCheck, "image");
        let snapshot = job.snapshot();
        assert_eq!(snapshot.current_phase, Some(HarnessSetupPhase::ImageCheck));
        assert_eq!(snapshot.phases.len(), 2);
        assert!(snapshot.phases[0].finished_at.is_some());
        assert!(snapshot.phases[0].elapsed_ms.is_some());
        assert!(snapshot.phases[1].finished_at.is_none());
    }

    #[test]
    fn machine_start_retry_predicate_only_retries_first_container_attempt() {
        let job = Arc::new(LaunchJob::new(
            uuid::Uuid::new_v4().to_string(),
            WorkspaceId::new(),
        ));
        let _ = job.transition_phase(HarnessSetupPhase::MachineStartOrInit, "starting machine");

        assert!(should_retry_machine_start_failure(&job, 1, false));
        assert!(!should_retry_machine_start_failure(&job, 2, false));
        assert!(!should_retry_machine_start_failure(&job, 1, true));
    }

    #[test]
    fn machine_start_retry_predicate_disables_retry_for_host_mode() {
        let job = Arc::new(LaunchJob::new(
            uuid::Uuid::new_v4().to_string(),
            WorkspaceId::new(),
        ));
        assert!(!should_retry_machine_start_failure(&job, 1, true));
    }

    #[test]
    fn machine_start_retry_predicate_requires_machine_start_phase() {
        let job = Arc::new(LaunchJob::new(
            uuid::Uuid::new_v4().to_string(),
            WorkspaceId::new(),
        ));
        let _ = job.transition_phase(HarnessSetupPhase::ImageCheck, "checking image");
        assert!(!should_retry_machine_start_failure(&job, 1, false));
    }

    #[test]
    fn format_error_chain_includes_context_and_cause() {
        let err = anyhow::anyhow!("inner").context("outer");
        assert_eq!(format_error_chain(&err), "outer: inner");
    }

    #[test]
    fn format_error_chain_marks_empty_cause() {
        #[derive(Debug)]
        struct EmptyCause;
        impl std::fmt::Display for EmptyCause {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "")
            }
        }
        impl std::error::Error for EmptyCause {}

        let err = anyhow::Error::new(EmptyCause).context("container runtime failed");
        assert_eq!(
            format_error_chain(&err),
            "container runtime failed: <empty error cause>"
        );
    }

    #[tokio::test]
    async fn concurrent_launch_start_is_deduplicated() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let coordinator = test_coordinator(data_dir.path().to_path_buf());
        let workspace = test_workspace(WorkspaceId::new());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let barrier = Arc::new(Barrier::new(3));

        let coordinator_a = Arc::clone(&coordinator);
        let workspace_a = workspace.clone();
        let settings_a = settings.clone();
        let barrier_a = Arc::clone(&barrier);
        let start_a = tokio::spawn(async move {
            barrier_a.wait().await;
            coordinator_a
                .start_workspace_launch(
                    workspace_a,
                    settings_a,
                    "http://127.0.0.1:4399".to_string(),
                )
                .await
        });

        let coordinator_b = Arc::clone(&coordinator);
        let workspace_b = workspace.clone();
        let settings_b = settings.clone();
        let barrier_b = Arc::clone(&barrier);
        let start_b = tokio::spawn(async move {
            barrier_b.wait().await;
            coordinator_b
                .start_workspace_launch(
                    workspace_b,
                    settings_b,
                    "http://127.0.0.1:4399".to_string(),
                )
                .await
        });

        barrier.wait().await;
        let first = start_a.await.expect("first launch task failed");
        let second = start_b.await.expect("second launch task failed");
        assert_eq!(first.job_id, second.job_id);
        assert_eq!(first.workspace_id, workspace.id.0.to_string());
        assert_eq!(second.workspace_id, workspace.id.0.to_string());
    }

    #[tokio::test]
    async fn subscribe_launch_returns_terminal_snapshot_when_job_is_done() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let coordinator = test_coordinator(data_dir.path().to_path_buf());
        let workspace_id = WorkspaceId::new();
        let job_id = uuid::Uuid::new_v4().to_string();
        let job = Arc::new(LaunchJob::new(job_id.clone(), workspace_id));

        {
            let mut inner = coordinator.inner.lock().await;
            inner
                .running_launch_by_workspace
                .insert(workspace_id, job_id.clone());
            inner.launch_jobs.insert(job_id.clone(), Arc::clone(&job));
            inner.launch_history.push_back(job_id.clone());
        }

        let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
        let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
            snapshot: terminal.snapshot,
        });
        coordinator
            .clear_running_launch(workspace_id, &job_id)
            .await;

        let (snapshot, mut rx) = coordinator
            .subscribe_launch(&job_id)
            .await
            .expect("missing launch job");
        assert_eq!(snapshot.state, ExecutionLaunchState::Ready);
        assert!(matches!(
            rx.try_recv(),
            Ok(ExecutionLaunchStreamEvent::LaunchComplete { .. })
                | Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn subscribe_launch_receiver_gets_terminal_event_after_running_snapshot() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let coordinator = test_coordinator(data_dir.path().to_path_buf());
        let workspace_id = WorkspaceId::new();
        let job_id = uuid::Uuid::new_v4().to_string();
        let job = Arc::new(LaunchJob::new(job_id.clone(), workspace_id));

        {
            let mut inner = coordinator.inner.lock().await;
            inner
                .running_launch_by_workspace
                .insert(workspace_id, job_id.clone());
            inner.launch_jobs.insert(job_id.clone(), Arc::clone(&job));
            inner.launch_history.push_back(job_id.clone());
        }

        let (snapshot, mut rx) = coordinator
            .subscribe_launch(&job_id)
            .await
            .expect("missing launch job");
        assert_eq!(snapshot.state, ExecutionLaunchState::Running);

        let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
        let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
            snapshot: terminal.snapshot,
        });
        coordinator
            .clear_running_launch(workspace_id, &job_id)
            .await;

        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for terminal launch event")
            .expect("launch event channel closed");
        assert!(matches!(
            event,
            ExecutionLaunchStreamEvent::LaunchComplete { .. }
        ));
    }
}
