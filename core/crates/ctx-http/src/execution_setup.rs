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
use ctx_harness_setup::{
    HarnessSetupDownloadStatus, HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase,
    HarnessSetupProgressUpdate,
};

use crate::ops_events::{OpsEvent, OpsEvents};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::settings::{ExecutionMode, ExecutionSettings};
use ctx_workspace_runtime::HarnessRuntimeManager;

mod launch_state;
mod progress;
mod startup_prewarm;
#[cfg(test)]
mod tests;
mod warmup_coordination;

use launch_state::{
    seed_runtime_prewarm_initial_state, seed_workspace_launch_initial_state, CoordinatorState,
    LaunchJob, LaunchTerminalMutation, PrewarmGate, StartupPrewarmMetadata,
};
use progress::{
    bundled_image_fingerprint, format_error_chain, format_ts, needs_prewarm,
    normalize_container_engine_ready_for_gate, phase_label, read_prewarm_metadata,
    write_prewarm_metadata, LaunchObserver,
};
use warmup_coordination::{
    DefaultWarmupOperations, LaunchPrewarmCoordinator, PrewarmJobRegistry, RequestedPrewarmScope,
    SharedPrewarmLaunchJob, SharedWarmupOperations,
};

const JOB_LOG_CAP: usize = 400;
const JOB_HISTORY_CAP: usize = 128;
const LAUNCH_EVENT_CHANNEL_CAP: usize = 256;

fn runtime_prewarm_ready_phase_message(
    runtime_requested: bool,
    runtime_kind: &crate::settings::ContainerRuntimeKind,
    launch_ready: bool,
) -> &'static str {
    if runtime_requested {
        ctx_harness_runtime::runtime_prewarm_ready_message(runtime_kind, launch_ready)
    } else {
        "container builder is ready"
    }
}

fn lock_or_recover<'a, T>(mutex: &'a StdMutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod ready_message_tests {
    use super::runtime_prewarm_ready_phase_message;
    use crate::settings::ContainerRuntimeKind;

    #[test]
    fn runtime_prewarm_ready_phase_message_uses_runtime_specific_semantics() {
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                true,
                &ContainerRuntimeKind::SharedVmContainer,
                false,
            ),
            "shared VM runtime artifacts are ready; launch image loads when the shared VM starts"
        );
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                true,
                &ContainerRuntimeKind::SharedVmContainer,
                true,
            ),
            "shared VM substrate and launch image are ready"
        );
        assert_eq!(
            runtime_prewarm_ready_phase_message(true, &ContainerRuntimeKind::NativeContainer, true,),
            "local sandbox runtime and launch image are ready"
        );
    }

    #[test]
    fn runtime_prewarm_ready_phase_message_preserves_builder_message() {
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                false,
                &ContainerRuntimeKind::SharedVmContainer,
                false,
            ),
            "container builder is ready"
        );
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
    LaunchReady,
    Builder,
    All,
}

impl RuntimePrewarmScope {
    fn includes_runtime(self) -> bool {
        matches!(self, Self::Runtime | Self::LaunchReady | Self::All)
    }

    fn includes_builder(self) -> bool {
        matches!(self, Self::Builder | Self::All)
    }

    fn requires_launch_ready_runtime(self) -> bool {
        matches!(self, Self::LaunchReady | Self::All)
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
// EXCEPTION: clippy::enum_variant_names — these stable serde tags are part of the
// execution launch stream API, so the shared `Launch*` prefix is deliberate.
#[allow(clippy::enum_variant_names)]
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
        let operations = Arc::new(DefaultWarmupOperations::new(
            data_root.clone(),
            ops_events.clone(),
        ));
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

    pub fn spawn_startup_prewarm(self: &Arc<Self>, execution: ExecutionSettings) {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            #[cfg(test)]
            // Tests temporarily rebind sandbox CLI process env vars, so startup prewarm
            // must serialize with the shared test lock before it can observe them.
            let _sandbox_cli_env_test_lock = crate::test_support::sandbox_cli_env_test_lock()
                .lock()
                .await;
            coordinator.run_startup_prewarm(execution).await;
        });
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(crate) async fn record_startup_prewarm_error(&self, message: String) {
        let attempted_at = format_ts(Utc::now());
        let snapshot = StartupPrewarmSnapshot {
            state: StartupPrewarmState::Error,
            target_image: String::new(),
            needs_prewarm: false,
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
            if matches!(settings.mode, ExecutionMode::Host) {
                seed_workspace_launch_initial_state(job.as_ref(), &settings);
            } else {
                let _ = job.transition_phase(
                    HarnessSetupPhase::MachineCheck,
                    "checking container runtime",
                );
            }
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
                if existing.request_scope(scope) {
                    return existing.snapshot();
                }
                inner
                    .prewarm_jobs
                    .remove_if_current(existing.key(), &existing);
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
        #[cfg(test)]
        eprintln!("run_workspace_launch: begin workspace={:?}", workspace.id);
        let launch_started = std::time::Instant::now();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let run_result = if is_host_mode {
            Ok(())
        } else {
            async {
                // Any compatible runtime prewarm can still save duplicate image work.
                // We check launch readiness after the join before deciding which launch path
                // to take.
                let join_shared_runtime = self.prewarm.runtime_is_running(&settings, false).await;
                #[cfg(test)]
                eprintln!("run_workspace_launch: join_shared_runtime={join_shared_runtime}");

                if join_shared_runtime {
                    let reusable_container_exists = self
                        .harness
                        .workspace_container_exists(workspace.id)
                        .await
                        .context("failed to probe existing workspace container")?;
                    #[cfg(test)]
                    eprintln!(
                        "run_workspace_launch: reusable_container_exists={reusable_container_exists}"
                    );
                    if reusable_container_exists {
                        #[cfg(test)]
                        eprintln!("run_workspace_launch: ensure_workspace_container_with_observer");
                        return self
                            .harness
                            .ensure_workspace_container_with_observer(
                                &workspace,
                                &settings,
                                &daemon_url,
                                Some(&observer),
                            )
                            .await
                            .context("container runtime failed");
                    }

                    let _runtime_activity = self.harness.begin_runtime_operation();
                    self.harness
                        .ensure_container_machine_ready(&settings.container, Some(&observer))
                        .await
                        .context("sandbox runtime unavailable and execution mode is sandbox")?;

                    let joined_shared_runtime = match self
                        .prewarm
                        .attach_runtime_if_running(&settings, false, Some(&observer))
                        .await
                    {
                        Ok(joined) => joined,
                        Err(err) => {
                            observer.on_log(
                                HarnessSetupPhase::MachineStartOrInit,
                                HarnessSetupLogLevel::Warn,
                                &format!(
                                    "shared runtime warmup failed, continuing with direct launch: {}",
                                    format_error_chain(&err)
                                ),
                            );
                            false
                        }
                    };

                    if joined_shared_runtime {
                        let launch_ready = ctx_harness_runtime::selected_runtime_launch_ready(
                            &self.data_root,
                            &settings.container,
                        )
                        .await
                        .context("failed to inspect container runtime after shared warmup")?;
                        if launch_ready {
                            return self
                                .harness
                                .ensure_workspace_container_after_runtime_ready_with_observer(
                                    &workspace,
                                    &settings,
                                    &daemon_url,
                                    Some(&observer),
                                )
                                .await
                                .context("container runtime failed");
                        }
                    }

                    self.harness
                        .ensure_workspace_container_after_machine_ready_with_observer(
                            &workspace,
                            &settings,
                            &daemon_url,
                            Some(&observer),
                        )
                        .await
                        .context("container runtime failed")
                } else {
                    #[cfg(test)]
                    eprintln!("run_workspace_launch: direct ensure_workspace_container_with_observer");
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
            }
            .await
        };

        match run_result {
            Ok(()) => {
                #[cfg(test)]
                eprintln!("run_workspace_launch: success");
                if !matches!(settings.mode, ExecutionMode::Host) {
                    self.refresh_startup_prewarm_metadata_after_successful_container_launch(
                        &settings.container,
                    )
                    .await;
                    self.emit_phase(
                        &job,
                        HarnessSetupPhase::Ready,
                        ctx_harness_runtime::workspace_launch_ready_message(
                            &settings.container.runtime,
                        ),
                    );
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
                #[cfg(test)]
                eprintln!("run_workspace_launch: error={err:#}");
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
        let runtime_target = ctx_harness_runtime::runtime_prewarm_target(&settings.container);
        if is_host_mode {
            if let Some(terminal) = shared_job.complete_ready() {
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            self.clear_running_prewarm(&shared_job).await;
            return;
        }

        let run_result: Result<RequestedPrewarmScope> = async {
            let mut runtime_ready = false;
            let mut launch_ready = false;
            let mut builder_ready = false;

            loop {
                let requested_scope = shared_job.requested_scope();
                if requested_scope.requires_launch_ready_runtime() && !launch_ready {
                    if !ctx_harness_runtime::local_runtime_available(
                        &self.data_root,
                        &settings.container.runtime,
                    ) {
                        return Err(anyhow::anyhow!("local sandbox runtime unavailable"));
                    }
                    let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
                    self.prewarm
                        .ensure_runtime(&settings, true, Some(&observer))
                        .await?;
                    self.validate_runtime_prewarm_completion(&settings, &runtime_target, true)
                        .await?;
                    runtime_ready = true;
                    launch_ready = true;
                    continue;
                }

                if requested_scope.runtime_requested() && !runtime_ready {
                    if !ctx_harness_runtime::local_runtime_available(
                        &self.data_root,
                        &settings.container.runtime,
                    ) {
                        return Err(anyhow::anyhow!("local sandbox runtime unavailable"));
                    }
                    let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
                    self.prewarm
                        .ensure_runtime(&settings, false, Some(&observer))
                        .await?;
                    self.validate_runtime_prewarm_completion(&settings, &runtime_target, false)
                        .await?;
                    runtime_ready = true;
                    continue;
                }

                if requested_scope.builder_requested() && !builder_ready {
                    self.wait_for_builder_completion(observer.clone()).await?;
                    builder_ready = true;
                    continue;
                }

                if let Some(requested_scope) = shared_job
                    .reserve_ready_completion_if_scope_satisfied(
                        runtime_ready,
                        launch_ready,
                        builder_ready,
                    )
                {
                    return Ok(requested_scope);
                }

                tokio::task::yield_now().await;
            }
        }
        .await;

        match run_result {
            Ok(requested_scope) => {
                let runtime_kind = &settings.container.runtime;
                let ready_message = runtime_prewarm_ready_phase_message(
                    requested_scope.runtime_requested(),
                    runtime_kind,
                    requested_scope.requires_launch_ready_runtime(),
                );
                self.emit_phase(&job, HarnessSetupPhase::Ready, ready_message);
                let terminal = shared_job.mark_reserved_ready_terminal();
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            Err(err) => {
                self.finish_runtime_prewarm_error(shared_job, job, launch_started, err)
                    .await;
                return;
            }
        }

        self.clear_running_prewarm(&shared_job).await;
    }

    async fn validate_runtime_prewarm_completion(
        &self,
        settings: &ExecutionSettings,
        runtime_target: &str,
        requires_launch_ready_runtime: bool,
    ) -> Result<()> {
        if requires_launch_ready_runtime {
            match ctx_harness_runtime::selected_runtime_launch_readiness_state(
                &self.data_root,
                &settings.container,
            )
            .await
            {
                Ok((true, true)) => Ok(()),
                Ok((vm_ready, image_ready)) => Err(anyhow::anyhow!(
                    ctx_harness_runtime::launch_ready_gap_message(
                        settings.container.runtime.clone(),
                        runtime_target,
                        vm_ready,
                        image_ready,
                    )
                )),
                Err(err) => Err(err),
            }
        } else {
            match ctx_harness_runtime::selected_runtime_state(&self.data_root, &settings.container).await
            {
                Ok((machine_ready, image_present)) if machine_ready && image_present => Ok(()),
                Ok((machine_ready, _image_present)) if machine_ready => Err(anyhow::anyhow!(
                    "runtime prewarm completed but runtime target '{runtime_target}' is still unavailable in the local sandbox runtime"
                )),
                Ok((_machine_ready, _image_present)) => Err(anyhow::anyhow!(
                    "runtime prewarm downloaded startup artifacts for '{runtime_target}', but the local sandbox runtime still needs first-launch startup"
                )),
                Err(err) => Err(err),
            }
        }
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
}
