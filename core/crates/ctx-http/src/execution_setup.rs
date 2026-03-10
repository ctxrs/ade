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
    self, HarnessRuntimeManager, HarnessSetupDownloadStatus, HarnessSetupLogLevel,
    HarnessSetupObserver, HarnessSetupPhase, HarnessSetupProgressUpdate,
};
use crate::ops_events::{OpsEvent, OpsEvents};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::settings::{ExecutionMode, ExecutionSettings};

mod warmup_coordination;

use warmup_coordination::{
    DefaultWarmupOperations, LaunchPrewarmCoordinator, PrewarmJobRegistry, SharedPrewarmLaunchJob,
    SharedWarmupOperations,
};

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
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<HarnessSetupPhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_download: Option<HarnessSetupDownloadStatus>,
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
    prewarm_jobs: PrewarmJobRegistry,
    startup: StartupPrewarmSnapshot,
}

impl CoordinatorState {
    fn remember_launch_job(&mut self, job: Arc<LaunchJob>) {
        self.launch_jobs
            .insert(job.job_id.clone(), Arc::clone(&job));
        self.launch_history.push_back(job.job_id.clone());
        while self.launch_history.len() > JOB_HISTORY_CAP {
            let Some(old_id) = self.launch_history.pop_front() else {
                break;
            };
            if !self.is_job_running(&old_id) {
                self.launch_jobs.remove(&old_id);
            }
        }
    }

    fn is_job_running(&self, job_id: &str) -> bool {
        self.running_launch_by_workspace
            .values()
            .any(|active_id| active_id == job_id)
            || self.prewarm_jobs.contains_job_id(job_id)
    }
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
    current_step_label: Option<String>,
    active_download: Option<HarnessSetupDownloadStatus>,
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
            current_step_label: None,
            active_download: None,
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
        let now = Utc::now();
        let eta_ms = estimate_remaining_ms(self, now);
        let progress_pct = project_progress_pct(self, now, eta_ms);
        ExecutionLaunchSnapshot {
            job_id: job_id.to_string(),
            workspace_id: workspace_id.0.to_string(),
            kind: self.kind,
            state: self.state,
            created_at: format_ts(self.created_at),
            started_at: format_ts(self.started_at),
            updated_at: format_ts(now),
            finished_at: self.finished_at.map(format_ts),
            current_phase: self.current_phase,
            current_step_label: self.current_step_label.clone(),
            progress_pct,
            eta_ms,
            active_download: self.active_download.clone(),
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
        Self::new_with_kind(job_id, workspace_id, ExecutionSetupJobKind::WorkspaceLaunch)
    }

    fn new_with_kind(
        job_id: String,
        workspace_id: WorkspaceId,
        kind: ExecutionSetupJobKind,
    ) -> Self {
        let (tx, _) = broadcast::channel(LAUNCH_EVENT_CHANNEL_CAP);
        Self {
            job_id,
            workspace_id,
            tx,
            inner: StdMutex::new(LaunchJobInner::new(kind)),
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

    fn is_terminal(&self) -> bool {
        let inner = lock_or_recover(&self.inner, "launch job");
        inner.state != ExecutionLaunchState::Running
    }

    fn transition_phase(&self, phase: HarnessSetupPhase, message: &str) -> LaunchMutation {
        let now = Utc::now();
        let mut inner = lock_or_recover(&self.inner, "launch job");
        let mut completed_phase = None;
        let mut snapshot_changed = false;
        let previous_phase = inner.current_phase;
        let previous_label = inner.current_step_label.clone();
        if inner.current_phase != Some(phase) {
            completed_phase = inner.close_current_phase(now);
            inner.current_phase = Some(phase);
            if phase != HarnessSetupPhase::ArtifactDownload {
                inner.active_download = None;
            }
            inner.phases.push(LaunchPhaseRecord {
                phase,
                started_at: now,
                finished_at: None,
                elapsed_ms: None,
            });
            snapshot_changed = true;
        }
        let next_label = message.trim();
        let label = if next_label.is_empty() {
            None
        } else {
            Some(next_label.to_string())
        };
        if inner.current_step_label != label {
            inner.current_step_label = label;
            snapshot_changed = true;
        }
        let should_log = previous_phase != Some(phase)
            || previous_label.as_deref().map(str::trim) != Some(next_label);
        let line = if should_log {
            Some(inner.push_log(phase, HarnessSetupLogLevel::Info, message, now))
        } else {
            None
        };
        let snapshot = inner.snapshot(&self.job_id, self.workspace_id);
        LaunchMutation {
            line,
            snapshot,
            snapshot_changed,
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
            snapshot_changed: false,
            completed_phase: None,
        }
    }

    fn set_progress(&self, progress: HarnessSetupProgressUpdate) -> LaunchMutation {
        let mut inner = lock_or_recover(&self.inner, "launch job");
        inner.active_download = progress.active_download;
        let snapshot = inner.snapshot(&self.job_id, self.workspace_id);
        LaunchMutation {
            line: None,
            snapshot,
            snapshot_changed: true,
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
        inner.active_download = None;
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
    snapshot_changed: bool,
    completed_phase: Option<CompletedPhase>,
}

#[derive(Debug)]
struct LaunchTerminalMutation {
    snapshot: ExecutionLaunchSnapshot,
    completed_phase: Option<CompletedPhase>,
}

fn seed_workspace_launch_initial_state(job: &LaunchJob, settings: &ExecutionSettings) {
    if matches!(settings.mode, ExecutionMode::Host) {
        let _ = job.transition_phase(
            HarnessSetupPhase::Ready,
            "host execution mode selected; container launch skipped",
        );
    } else {
        let _ = job.transition_phase(
            HarnessSetupPhase::MachineCheck,
            "requesting shared container readiness",
        );
    }
}

fn seed_runtime_prewarm_initial_state(job: &LaunchJob, settings: &ExecutionSettings) {
    if matches!(settings.mode, ExecutionMode::Host) {
        let _ = job.transition_phase(
            HarnessSetupPhase::Ready,
            "host execution mode selected; runtime prewarm skipped",
        );
    } else {
        let _ = job.transition_phase(
            HarnessSetupPhase::MachineCheck,
            "requesting shared container readiness",
        );
    }
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
    prewarm: LaunchPrewarmCoordinator,
}

impl ExecutionSetupCoordinator {
    pub fn new(
        data_root: PathBuf,
        harness: Arc<HarnessRuntimeManager>,
        perf_telemetry: PerfTelemetry,
        ops_events: OpsEvents,
    ) -> Self {
        let operations = Arc::new(DefaultWarmupOperations::new(data_root.clone()));
        Self::new_with_operations(data_root, harness, perf_telemetry, ops_events, operations)
    }

    fn new_with_operations(
        data_root: PathBuf,
        harness: Arc<HarnessRuntimeManager>,
        perf_telemetry: PerfTelemetry,
        ops_events: OpsEvents,
        operations: Arc<dyn SharedWarmupOperations>,
    ) -> Self {
        Self {
            prewarm: LaunchPrewarmCoordinator::new(operations),
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
            seed_workspace_launch_initial_state(job.as_ref(), &settings);
            let snapshot = job.snapshot();

            inner
                .running_launch_by_workspace
                .insert(workspace.id, job_id.clone());
            inner.remember_launch_job(Arc::clone(&job));

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
        let (shared_job, snapshot) = {
            let mut inner = self.inner.lock().await;
            if let Some(existing) = inner.prewarm_jobs.find_compatible(&settings, scope) {
                let snapshot = existing.snapshot();
                return snapshot;
            }

            let job = Arc::new(SharedPrewarmLaunchJob::new(
                uuid::Uuid::new_v4().to_string(),
                &settings,
                scope,
            ));
            let snapshot = job.snapshot();
            inner.prewarm_jobs.insert(Arc::clone(&job));
            inner.remember_launch_job(job.job());
            (job, snapshot)
        };

        let launch_job = shared_job.job();
        let _ = launch_job
            .tx
            .send(ExecutionLaunchStreamEvent::LaunchSnapshot {
                snapshot: snapshot.clone(),
            });

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator.run_runtime_prewarm(shared_job, settings).await;
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
        let run_result = if is_host_mode {
            Ok(())
        } else {
            let startup_running = {
                let inner = self.inner.lock().await;
                inner.startup.state == StartupPrewarmState::Running
            };
            if startup_running {
                match self
                    .prewarm
                    .ensure_runtime(&settings, Some(&observer))
                    .await
                    .context("container runtime warmup failed")
                {
                    Ok(()) => self
                        .harness
                        .ensure_workspace_container_after_runtime_ready_with_observer(
                            &workspace,
                            &settings,
                            &daemon_url,
                            Some(&observer),
                        )
                        .await
                        .context("container runtime failed"),
                    Err(err) => Err(err),
                }
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
        shared_job: Arc<SharedPrewarmLaunchJob>,
        settings: ExecutionSettings,
    ) {
        let launch_started = std::time::Instant::now();
        let job = shared_job.job();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let run_result = if is_host_mode {
            Ok(())
        } else if shared_job.runtime_requested() {
            if !harness_runtime::container_runtime_available(&self.data_root) {
                Err(anyhow::anyhow!("container runtime unavailable"))
            } else {
                match self
                    .prewarm
                    .ensure_runtime(&settings, Some(&observer))
                    .await
                {
                    Ok(()) => {
                        if shared_job.builder_requested() {
                            match self.wait_for_builder_completion(observer.clone()).await {
                                Ok(()) => {}
                                Err(err) => {
                                    return self
                                        .finish_runtime_prewarm_error(
                                            shared_job,
                                            job,
                                            launch_started,
                                            err,
                                        )
                                        .await
                                }
                            }
                        }
                        Ok(())
                    }
                    Err(err) => Err(err),
                }
            }
        } else if shared_job.builder_requested() {
            self.prewarm.ensure_builder(Some(&observer)).await
        } else {
            Ok(())
        };

        match run_result {
            Ok(()) => {
                if !matches!(settings.mode, ExecutionMode::Host) {
                    let ready_message = if shared_job.runtime_requested() {
                        "container runtime is ready"
                    } else {
                        "container builder is ready"
                    };
                    self.emit_phase(&job, HarnessSetupPhase::Ready, ready_message);
                }
                if let Some(terminal) = shared_job.complete_ready() {
                    if let Some(completed) = terminal.completed_phase {
                        self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                    }
                    let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                        snapshot: terminal.snapshot.clone(),
                    });
                    self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
                }
            }
            Err(err) => {
                self.finish_runtime_prewarm_error(shared_job, job, launch_started, err)
                    .await;
                return;
            }
        }

        self.clear_running_prewarm(&shared_job).await;
    }

    async fn finish_runtime_prewarm_error(
        &self,
        shared_job: Arc<SharedPrewarmLaunchJob>,
        job: Arc<LaunchJob>,
        launch_started: std::time::Instant,
        err: anyhow::Error,
    ) {
        let message = format_error_chain(&err);
        let phase = job.current_phase().unwrap_or(HarnessSetupPhase::ImageLoad);
        self.emit_log(&job, phase, HarnessSetupLogLevel::Error, &message);
        if let Some(terminal) = shared_job.complete_error(message.clone()) {
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
        self.clear_running_prewarm(&shared_job).await;
    }

    async fn wait_for_builder_completion(self: &Arc<Self>, observer: LaunchObserver) -> Result<()> {
        let coordinator = Arc::clone(self);
        let builder_observer = observer;
        tokio::spawn(async move {
            coordinator
                .prewarm
                .ensure_builder(Some(&builder_observer))
                .await
        })
        .await
        .map_err(|err| anyhow::anyhow!("builder warmup task join failed: {err}"))?
    }

    async fn clear_running_prewarm(&self, shared_job: &Arc<SharedPrewarmLaunchJob>) {
        let mut inner = self.inner.lock().await;
        inner
            .prewarm_jobs
            .remove_if_current(shared_job.key(), shared_job);
    }

    fn emit_phase(&self, job: &Arc<LaunchJob>, phase: HarnessSetupPhase, message: &str) {
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

    fn emit_progress(&self, job: &Arc<LaunchJob>, progress: HarnessSetupProgressUpdate) {
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

    fn emit_log(
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

        if let Err(err) = self.prewarm.prefetch_runtime_artifacts(&exec, None).await {
            let message = format_error_chain(&err);
            tracing::warn!("startup artifact prefetch failed: {message}");
            let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
            event.meta = Some(json!({"image": image, "error": message}));
            self.ops_events.emit(event);

            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Error,
                target_image: image,
                needs_prewarm: true,
                machine_ready: false,
                image_present: false,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at),
                last_success_at: None,
                error: Some(err.to_string()),
            };
            self.set_startup_snapshot(snapshot).await;
            return;
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

        match self
            .prewarm
            .ensure_scope(&exec, RuntimePrewarmScope::Runtime, None)
            .await
        {
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
        let machine_ready = normalize_podman_engine_ready_for_gate(
            harness_runtime::podman_engine_ready(&self.data_root).await,
        )?;
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

#[derive(Clone)]
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

    fn on_progress(&self, progress: HarnessSetupProgressUpdate) {
        self.coordinator.emit_progress(&self.job, progress);
    }
}

fn phase_label(phase: HarnessSetupPhase) -> &'static str {
    match phase {
        HarnessSetupPhase::ArtifactDownload => "artifact_download",
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

fn phase_budget_ms(kind: ExecutionSetupJobKind, phase: HarnessSetupPhase) -> u64 {
    match phase {
        HarnessSetupPhase::ArtifactDownload => 150_000,
        HarnessSetupPhase::MachineCheck => 2_000,
        HarnessSetupPhase::MachineStartOrInit => 25_000,
        HarnessSetupPhase::ImageCheck => 1_000,
        HarnessSetupPhase::ImageLoad => 5_000,
        HarnessSetupPhase::ContainerCheck => {
            if kind == ExecutionSetupJobKind::WorkspaceLaunch {
                500
            } else {
                0
            }
        }
        HarnessSetupPhase::ContainerStartOrCreate => {
            if kind == ExecutionSetupJobKind::WorkspaceLaunch {
                800
            } else {
                0
            }
        }
        HarnessSetupPhase::RuntimeNetworkSetup => {
            if kind == ExecutionSetupJobKind::WorkspaceLaunch {
                1_000
            } else {
                0
            }
        }
        HarnessSetupPhase::Ready => 0,
    }
}

fn remaining_future_phase_budget_ms(kind: ExecutionSetupJobKind, phase: HarnessSetupPhase) -> u64 {
    match kind {
        ExecutionSetupJobKind::StartupPrewarm => match phase {
            HarnessSetupPhase::ArtifactDownload | HarnessSetupPhase::MachineCheck => {
                phase_budget_ms(kind, HarnessSetupPhase::MachineStartOrInit)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageLoad)
            }
            HarnessSetupPhase::MachineStartOrInit => {
                phase_budget_ms(kind, HarnessSetupPhase::ImageCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageLoad)
            }
            HarnessSetupPhase::ImageCheck => phase_budget_ms(kind, HarnessSetupPhase::ImageLoad),
            HarnessSetupPhase::ImageLoad
            | HarnessSetupPhase::ContainerCheck
            | HarnessSetupPhase::ContainerStartOrCreate
            | HarnessSetupPhase::RuntimeNetworkSetup
            | HarnessSetupPhase::Ready => 0,
        },
        ExecutionSetupJobKind::WorkspaceLaunch => match phase {
            HarnessSetupPhase::ArtifactDownload | HarnessSetupPhase::MachineCheck => {
                phase_budget_ms(kind, HarnessSetupPhase::MachineStartOrInit)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageLoad)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerStartOrCreate)
                    + phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::MachineStartOrInit => {
                phase_budget_ms(kind, HarnessSetupPhase::ImageCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ImageLoad)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerStartOrCreate)
                    + phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::ImageCheck => {
                phase_budget_ms(kind, HarnessSetupPhase::ImageLoad)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerStartOrCreate)
                    + phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::ImageLoad => {
                phase_budget_ms(kind, HarnessSetupPhase::ContainerCheck)
                    + phase_budget_ms(kind, HarnessSetupPhase::ContainerStartOrCreate)
                    + phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::ContainerCheck => {
                phase_budget_ms(kind, HarnessSetupPhase::ContainerStartOrCreate)
                    + phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::ContainerStartOrCreate => {
                phase_budget_ms(kind, HarnessSetupPhase::RuntimeNetworkSetup)
            }
            HarnessSetupPhase::RuntimeNetworkSetup | HarnessSetupPhase::Ready => 0,
        },
    }
}

fn running_phase_elapsed_ms(inner: &LaunchJobInner, now: DateTime<Utc>) -> u64 {
    let Some(current_phase) = inner.current_phase else {
        return 0;
    };
    let started_at = inner
        .phases
        .iter()
        .rev()
        .find(|phase| phase.phase == current_phase && phase.finished_at.is_none())
        .map(|phase| phase.started_at)
        .unwrap_or(inner.started_at);
    now.signed_duration_since(started_at)
        .num_milliseconds()
        .max(0) as u64
}

fn estimate_remaining_ms(inner: &LaunchJobInner, now: DateTime<Utc>) -> Option<u64> {
    match inner.state {
        ExecutionLaunchState::Ready | ExecutionLaunchState::Error => return Some(0),
        ExecutionLaunchState::Running => {}
    }
    let current_phase = inner.current_phase?;
    let elapsed_ms = running_phase_elapsed_ms(inner, now);
    let current_remaining_ms = if current_phase == HarnessSetupPhase::ArtifactDownload {
        if let Some(download) = inner.active_download.as_ref() {
            match (download.total_bytes, download.bytes_per_sec) {
                (Some(total_bytes), Some(bytes_per_sec)) if bytes_per_sec > 0 => {
                    total_bytes
                        .saturating_sub(download.downloaded_bytes)
                        .saturating_mul(1000)
                        / bytes_per_sec
                }
                _ => phase_budget_ms(inner.kind, current_phase).saturating_sub(elapsed_ms),
            }
        } else {
            phase_budget_ms(inner.kind, current_phase).saturating_sub(elapsed_ms)
        }
    } else {
        phase_budget_ms(inner.kind, current_phase).saturating_sub(elapsed_ms)
    };
    Some(current_remaining_ms + remaining_future_phase_budget_ms(inner.kind, current_phase))
}

fn project_progress_pct(
    inner: &LaunchJobInner,
    now: DateTime<Utc>,
    eta_ms: Option<u64>,
) -> Option<u8> {
    match inner.state {
        ExecutionLaunchState::Ready => return Some(100),
        ExecutionLaunchState::Error => return Some(100),
        ExecutionLaunchState::Running => {}
    }
    let eta_ms = eta_ms?;
    let elapsed_ms = now
        .signed_duration_since(inner.started_at)
        .num_milliseconds()
        .max(0) as u64;
    let total = elapsed_ms.saturating_add(eta_ms);
    if total == 0 {
        return Some(0);
    }
    Some(
        ((elapsed_ms as f64 / total as f64) * 100.0)
            .round()
            .clamp(0.0, 99.0) as u8,
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

fn normalize_podman_engine_ready_for_gate(result: anyhow::Result<bool>) -> anyhow::Result<bool> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            if err
                .to_string()
                .to_ascii_lowercase()
                .contains("podman binary unavailable")
            {
                // Thin desktop bundles can start with no local podman binary yet; treat that as
                // "not ready" so startup prewarm proceeds to managed runtime install.
                return Ok(false);
            }
            Err(err)
        }
    }
}

#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::{Barrier, Notify};

    use crate::execution_setup::warmup_coordination::SharedWarmupOperations;
    use crate::harness_runtime::HarnessRuntimeManager;
    use crate::ops_events::OpsEvents;
    use crate::perf_telemetry::PerfTelemetry;
    use crate::settings::{ExecutionMode, ExecutionSettings, Settings};

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }

        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::podman_env_test_lock()
    }

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

    fn test_coordinator_with_operations(
        data_root: PathBuf,
        operations: Arc<dyn SharedWarmupOperations>,
    ) -> Arc<ExecutionSetupCoordinator> {
        Arc::new(ExecutionSetupCoordinator::new_with_operations(
            data_root.clone(),
            Arc::new(HarnessRuntimeManager::new(data_root.clone())),
            PerfTelemetry::new(data_root.clone()),
            OpsEvents::new(data_root),
            operations,
        ))
    }

    async fn init_settings_store(data_root: &Path) {
        let db_path = data_root.join("db").join("db.sqlite");
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("create db directory");
        }
        let store = Store::open_sqlite(&db_path, None)
            .await
            .expect("open sqlite store");
        crate::settings::save_settings(&store, &crate::settings::Settings::default())
            .await
            .expect("save default settings");
        store.close().await;
    }

    fn write_startup_prewarm_podman_shim(dir: &Path) -> PathBuf {
        let path = dir.join(if cfg!(windows) {
            "podman-startup-test.cmd"
        } else {
            "podman-startup-test.sh"
        });
        let script = if cfg!(windows) {
            "@echo off\r\nif \"%1\"==\"info\" (\r\n  >&2 echo engine unavailable\r\n  exit /b 125\r\n)\r\n>&2 echo unexpected podman invocation: %*\r\nexit /b 1\r\n"
        } else {
            "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  echo 'engine unavailable' >&2\n  exit 125\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n"
        };
        std::fs::write(&path, script).expect("write startup prewarm podman shim");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod startup prewarm podman shim");
        }
        path
    }

    async fn save_test_execution_settings(data_root: &Path, execution: ExecutionSettings) {
        let db_path = data_root.join("db").join("db.sqlite");
        let store = Store::open_sqlite(&db_path, None)
            .await
            .expect("open settings store");
        let settings = Settings {
            execution: Some(execution),
            ..Settings::default()
        };
        crate::settings::save_settings(&store, &settings)
            .await
            .expect("save settings");
        store.close().await;
    }
    #[derive(Default)]
    struct BlockingWarmupOperations {
        runtime_runs: AtomicUsize,
        builder_runs: AtomicUsize,
        runtime_release: Notify,
        builder_release: Notify,
    }

    impl BlockingWarmupOperations {
        async fn wait_for_runtime_runs(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if self.runtime_runs.load(Ordering::SeqCst) >= expected {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("timed out waiting for runtime runs");
        }

        async fn wait_for_builder_runs(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if self.builder_runs.load(Ordering::SeqCst) >= expected {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("timed out waiting for builder runs");
        }

        fn release_runtime(&self) {
            self.runtime_release.notify_waiters();
        }

        fn release_builder(&self) {
            self.builder_release.notify_waiters();
        }
    }

    #[derive(Default)]
    struct UnexpectedRuntimeWarmupOperations {
        runtime_runs: AtomicUsize,
    }

    #[async_trait]
    impl SharedWarmupOperations for UnexpectedRuntimeWarmupOperations {
        async fn prefetch_runtime_artifacts(
            &self,
            _settings: ExecutionSettings,
            _observer: Option<&dyn HarnessSetupObserver>,
        ) -> Result<()> {
            Ok(())
        }

        async fn warm_runtime(
            &self,
            _settings: ExecutionSettings,
            _observer: Arc<dyn HarnessSetupObserver>,
        ) -> Result<()> {
            self.runtime_runs.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("unexpected runtime warmup")
        }

        async fn warm_builder(&self, _observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl SharedWarmupOperations for BlockingWarmupOperations {
        async fn prefetch_runtime_artifacts(
            &self,
            _settings: ExecutionSettings,
            _observer: Option<&dyn HarnessSetupObserver>,
        ) -> Result<()> {
            Ok(())
        }

        async fn warm_runtime(
            &self,
            _settings: ExecutionSettings,
            observer: Arc<dyn HarnessSetupObserver>,
        ) -> Result<()> {
            self.runtime_runs.fetch_add(1, Ordering::SeqCst);
            observer.on_phase(HarnessSetupPhase::MachineCheck, "warming runtime");
            self.runtime_release.notified().await;
            Ok(())
        }

        async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
            self.builder_runs.fetch_add(1, Ordering::SeqCst);
            observer.on_phase(HarnessSetupPhase::ImageLoad, "warming builder");
            self.builder_release.notified().await;
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingStartupWarmupOperations {
        prefetch_runs: AtomicUsize,
        runtime_runs: AtomicUsize,
        steps: StdMutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl SharedWarmupOperations for RecordingStartupWarmupOperations {
        async fn prefetch_runtime_artifacts(
            &self,
            _settings: ExecutionSettings,
            _observer: Option<&dyn HarnessSetupObserver>,
        ) -> Result<()> {
            self.prefetch_runs.fetch_add(1, Ordering::SeqCst);
            self.steps
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push("prefetch");
            Ok(())
        }

        async fn warm_runtime(
            &self,
            _settings: ExecutionSettings,
            observer: Arc<dyn HarnessSetupObserver>,
        ) -> Result<()> {
            self.runtime_runs.fetch_add(1, Ordering::SeqCst);
            self.steps
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push("runtime");
            observer.on_phase(HarnessSetupPhase::MachineCheck, "warming runtime");
            Ok(())
        }

        async fn warm_builder(&self, _observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
            Ok(())
        }
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
    fn normalize_podman_engine_ready_for_gate_treats_missing_binary_as_not_ready() {
        let value = normalize_podman_engine_ready_for_gate(Err(anyhow::anyhow!(
            "podman binary unavailable"
        )))
        .expect("missing binary should map to not-ready");
        assert!(!value);
    }

    #[test]
    fn normalize_podman_engine_ready_for_gate_preserves_other_errors() {
        let err = normalize_podman_engine_ready_for_gate(Err(anyhow::anyhow!("boom")));
        assert!(err.is_err());
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
    fn duplicate_phase_updates_do_not_append_duplicate_logs() {
        let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
        let _ = job.transition_phase(
            HarnessSetupPhase::ArtifactDownload,
            "downloading required artifacts",
        );
        let _ = job.transition_phase(
            HarnessSetupPhase::ArtifactDownload,
            "downloading required artifacts",
        );
        let snapshot = job.snapshot();
        assert_eq!(snapshot.logs.len(), 1);
        assert_eq!(
            snapshot.current_step_label.as_deref(),
            Some("downloading required artifacts")
        );
    }

    #[test]
    fn launch_snapshot_projects_download_eta_and_progress() {
        let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
        let _ = job.transition_phase(
            HarnessSetupPhase::ArtifactDownload,
            "downloading required artifacts",
        );
        let _ = job.set_progress(HarnessSetupProgressUpdate {
            phase: HarnessSetupPhase::ArtifactDownload,
            active_download: Some(HarnessSetupDownloadStatus {
                artifact: "Required artifacts".to_string(),
                downloaded_bytes: 400,
                total_bytes: Some(1000),
                bytes_per_sec: Some(100),
            }),
        });
        let snapshot = job.snapshot();
        assert_eq!(
            snapshot.current_phase,
            Some(HarnessSetupPhase::ArtifactDownload)
        );
        assert_eq!(
            snapshot.current_step_label.as_deref(),
            Some("downloading required artifacts")
        );
        assert!(snapshot.eta_ms.unwrap_or(0) >= 39_000);
        assert!(snapshot.progress_pct.unwrap_or(0) < 100);
        assert_eq!(
            snapshot
                .active_download
                .as_ref()
                .and_then(|value| value.total_bytes),
            Some(1000)
        );
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

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_prewarm_runs_runtime_warmup_for_cold_container_settings() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let data_dir = tempfile::tempdir().expect("tempdir");
        let podman_path = data_dir.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nexit 0\n",
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let _podman_available = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };
        save_test_execution_settings(data_dir.path(), settings).await;

        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let coordinator_task = Arc::clone(&coordinator);
        let startup = tokio::spawn(async move {
            coordinator_task.run_startup_prewarm().await;
        });

        ops.wait_for_runtime_runs(1).await;

        let running = coordinator.startup_status().await;
        assert_eq!(running.state, StartupPrewarmState::Running);
        assert!(!running.machine_ready);

        ops.release_runtime();
        startup.await.expect("startup prewarm task");

        let ready = coordinator.startup_status().await;
        assert_eq!(ready.state, StartupPrewarmState::Ready);
        assert!(ready.needs_prewarm);
        assert!(ready.machine_ready);
        assert!(ready.image_present);
    }

    #[tokio::test]
    async fn runtime_prewarm_reuses_background_all_job_and_waits_for_builder_tail_when_runtime_joins(
    ) {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let background = coordinator
            .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::All)
            .await;
        ops.wait_for_runtime_runs(1).await;

        let foreground = coordinator
            .start_runtime_prewarm(settings, RuntimePrewarmScope::Runtime)
            .await;

        assert_eq!(foreground.job_id, background.job_id);
        {
            let inner = coordinator.inner.lock().await;
            assert_eq!(inner.launch_jobs.len(), 1);
            assert_eq!(inner.launch_history.len(), 1);
        }

        ops.release_runtime();
        ops.wait_for_builder_runs(1).await;

        let still_running = coordinator
            .launch_status(&background.job_id)
            .await
            .expect("missing shared prewarm job");
        assert_eq!(still_running.state, ExecutionLaunchState::Running);

        ops.release_builder();

        let ready = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&background.job_id)
                    .await
                    .expect("missing shared prewarm job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shared all prewarm wait timed out");

        assert_eq!(ready.job_id, background.job_id);
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_launch_waits_for_running_startup_prewarm_without_duplicate_runtime_warmup() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let data_dir = tempfile::tempdir().expect("tempdir");
        let workspace_root = data_dir.path().join("ws");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let log_path = data_dir.path().join("podman-invocations.log");
        let podman_path = data_dir.path().join("podman.sh");
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: "ws".to_string(),
            root_path: workspace_root.to_string_lossy().to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        };
        let container_name = format!("ctx-harness-{}", workspace.id.0);
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let _podman_available = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            container: crate::settings::ContainerExecutionSettings {
                network_mode: crate::settings::ContainerNetworkMode::All,
                ..Default::default()
            },
        };
        save_test_execution_settings(data_dir.path(), settings.clone()).await;

        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let coordinator_task = Arc::clone(&coordinator);
        let startup = tokio::spawn(async move {
            coordinator_task.run_startup_prewarm().await;
        });

        ops.wait_for_runtime_runs(1).await;

        let snapshot = coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await;
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);

        ops.release_runtime();
        startup.await.expect("startup prewarm task");

        let ready = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let latest = coordinator
                    .launch_status(&snapshot.job_id)
                    .await
                    .expect("missing workspace launch job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                if latest.state == ExecutionLaunchState::Error {
                    panic!("workspace launch failed unexpectedly: {:?}", latest.error);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for joined launch readiness");

        assert_eq!(ready.state, ExecutionLaunchState::Ready);
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);

        let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
        assert!(
            log.contains(&format!("container exists {container_name}")),
            "expected reusable container check in log:\n{log}"
        );
        assert!(
            !log.contains("image exists"),
            "joined launch should not restart runtime/image probes for reusable containers:\n{log}"
        );
    }

    #[tokio::test]
    async fn builder_prewarm_reuses_background_all_job() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let background = coordinator
            .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::All)
            .await;
        ops.wait_for_runtime_runs(1).await;

        let builder_only = coordinator
            .start_runtime_prewarm(settings, RuntimePrewarmScope::Builder)
            .await;

        assert_eq!(builder_only.job_id, background.job_id);
        {
            let inner = coordinator.inner.lock().await;
            assert_eq!(inner.launch_jobs.len(), 1);
            assert_eq!(inner.launch_history.len(), 1);
        }
        assert_eq!(
            ops.builder_runs.load(Ordering::SeqCst),
            0,
            "builder-only join should not start a second builder warmup before the shared all job reaches builder work"
        );

        ops.release_runtime();
        ops.wait_for_builder_runs(1).await;
        ops.release_builder();

        let ready = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&background.job_id)
                    .await
                    .expect("missing shared prewarm job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for shared all/builder prewarm readiness");

        assert_eq!(ready.job_id, background.job_id);
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
        assert_eq!(ops.builder_runs.load(Ordering::SeqCst), 1);
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

    #[tokio::test]
    async fn runtime_prewarm_emits_initial_log_before_runtime_work_completes() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let coordinator = test_coordinator(data_dir.path().to_path_buf());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let snapshot = coordinator
            .start_runtime_prewarm(settings, RuntimePrewarmScope::Runtime)
            .await;
        assert_eq!(
            snapshot.current_phase,
            Some(HarnessSetupPhase::MachineCheck)
        );
        assert!(snapshot.logs.iter().any(|line| {
            line.phase == HarnessSetupPhase::MachineCheck
                && line.message == "requesting shared container readiness"
        }));

        let observed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&snapshot.job_id)
                    .await
                    .expect("missing launch job");
                if !latest.logs.is_empty() {
                    break latest;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for initial launch log");

        assert!(matches!(
            observed.current_phase,
            Some(HarnessSetupPhase::MachineCheck) | None
        ));
        assert!(observed.logs.iter().any(|line| {
            line.phase == HarnessSetupPhase::MachineCheck
                && (line.message == "requesting shared container readiness"
                    || line.message == "checking container runtime")
        }));
    }

    #[tokio::test]
    async fn builder_only_prewarm_skips_runtime_warmup_and_runtime_availability() {
        let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "0");
        let data_dir = tempfile::tempdir().expect("tempdir");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let snapshot = coordinator
            .start_runtime_prewarm(settings, RuntimePrewarmScope::Builder)
            .await;

        ops.wait_for_builder_runs(1).await;
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

        let running = coordinator
            .launch_status(&snapshot.job_id)
            .await
            .expect("missing builder-only prewarm job");
        assert_eq!(running.current_phase, Some(HarnessSetupPhase::ImageLoad));
        assert!(running.logs.iter().any(|line| {
            line.phase == HarnessSetupPhase::ImageLoad && line.message == "warming builder"
        }));

        ops.release_builder();

        let terminal = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&snapshot.job_id)
                    .await
                    .expect("missing builder-only prewarm job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for builder-only prewarm readiness");

        assert_eq!(terminal.state, ExecutionLaunchState::Ready);
        assert!(terminal.logs.iter().any(|line| {
            line.phase == HarnessSetupPhase::Ready && line.message == "container builder is ready"
        }));
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn startup_prewarm_prefetches_artifacts_and_does_not_skip_when_machine_is_not_ready() {
        let _serial = env_var_test_lock().lock().await;
        let data_dir = tempfile::tempdir().expect("tempdir");
        let podman_path = write_startup_prewarm_podman_shim(data_dir.path());
        let _test_podman = EnvVarGuard::unset("CTX_TEST_PODMAN_AVAILABLE");
        let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        init_settings_store(data_dir.path()).await;

        let ops = Arc::new(RecordingStartupWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

        coordinator.run_startup_prewarm().await;

        let snapshot = coordinator.startup_status().await;
        assert_eq!(snapshot.state, StartupPrewarmState::Ready);
        assert!(snapshot.needs_prewarm);
        assert!(snapshot.machine_ready);
        assert!(snapshot.image_present);
        assert_eq!(ops.prefetch_runs.load(Ordering::SeqCst), 1);
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
        assert_eq!(
            *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
            vec!["prefetch", "runtime"]
        );
    }

    #[tokio::test]
    async fn workspace_launch_is_not_blocked_by_background_runtime_prewarm_job() {
        let _serial = env_var_test_lock().lock().await;
        let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
        let data_dir = tempfile::tempdir().expect("tempdir");
        let ops = Arc::new(BlockingWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let prewarm_settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };
        let workspace = test_workspace(WorkspaceId::new());
        let host_settings = ExecutionSettings {
            mode: ExecutionMode::Host,
            ..ExecutionSettings::default()
        };

        let background = coordinator
            .start_runtime_prewarm(prewarm_settings, RuntimePrewarmScope::Runtime)
            .await;
        ops.wait_for_runtime_runs(1).await;

        let launch = coordinator
            .start_workspace_launch(
                workspace.clone(),
                host_settings,
                "http://127.0.0.1:4399".to_string(),
            )
            .await;

        let ready = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&launch.job_id)
                    .await
                    .expect("missing workspace launch job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                if latest.state == ExecutionLaunchState::Error {
                    panic!("workspace launch failed unexpectedly: {:?}", latest.error);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for workspace launch readiness");

        let background_snapshot = coordinator
            .launch_status(&background.job_id)
            .await
            .expect("missing background prewarm job");
        assert_eq!(background_snapshot.state, ExecutionLaunchState::Running);
        assert_eq!(ready.state, ExecutionLaunchState::Ready);
        assert_ne!(background.job_id, launch.job_id);

        ops.release_runtime();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_launch_reuses_running_container_without_runtime_prewarm_or_image_checks() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let data_dir = tempfile::tempdir().expect("tempdir");
        let workspace_root = data_dir.path().join("ws");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let log_path = data_dir.path().join("podman-invocations.log");
        let podman_path = data_dir.path().join("podman.sh");
        let ops = Arc::new(UnexpectedRuntimeWarmupOperations::default());
        let coordinator =
            test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: "ws".to_string(),
            root_path: workspace_root.to_string_lossy().to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        };
        let container_name = format!("ctx-harness-{}", workspace.id.0);
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            container: crate::settings::ContainerExecutionSettings {
                network_mode: crate::settings::ContainerNetworkMode::All,
                ..Default::default()
            },
        };

        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"exists\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let snapshot = coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await;

        let ready = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let latest = coordinator
                    .launch_status(&snapshot.job_id)
                    .await
                    .expect("missing workspace launch job");
                if latest.state == ExecutionLaunchState::Ready {
                    break latest;
                }
                if latest.state == ExecutionLaunchState::Error {
                    panic!("workspace launch failed unexpectedly: {:?}", latest.error);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out waiting for workspace launch readiness");

        assert_eq!(ready.state, ExecutionLaunchState::Ready);
        assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

        let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
        assert!(
            log.contains(&format!("container exists {container_name}")),
            "expected existing-container check in log:\n{log}"
        );
        assert!(
            log.contains(&format!(
                "container inspect --format {{{{.State.Running}}}} {container_name}"
            )),
            "expected running-container inspect in log:\n{log}"
        );
        assert!(
            !log.contains("image exists"),
            "workspace launch should not front-load image checks for reusable containers:\n{log}"
        );
    }

    #[tokio::test]
    async fn workspace_launch_emits_initial_log_before_runtime_work_completes() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let coordinator = test_coordinator(data_dir.path().to_path_buf());
        let workspace = test_workspace(WorkspaceId::new());
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        let snapshot = coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await;
        let observed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let latest = coordinator
                    .launch_status(&snapshot.job_id)
                    .await
                    .expect("missing launch job");
                if !latest.logs.is_empty() {
                    break latest;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for initial launch log");

        assert!(matches!(
            observed.current_phase,
            Some(HarnessSetupPhase::MachineCheck) | None
        ));
        assert!(observed.logs.iter().any(|line| {
            line.phase == HarnessSetupPhase::MachineCheck
                && (line.message == "requesting shared container readiness"
                    || line.message == "checking container runtime")
        }));
    }
}
