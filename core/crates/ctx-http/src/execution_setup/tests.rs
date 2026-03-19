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
use crate::test_support::{
    wait_for_execution_launch_terminal, write_running_container_podman_shim, TrackedExecutionLaunch,
};

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
        tokio::time::timeout(Duration::from_secs(10), async {
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
        tokio::time::timeout(Duration::from_secs(10), async {
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
    runtime_runs: AtomicUsize,
    steps: StdMutex<Vec<&'static str>>,
}

#[async_trait]
impl SharedWarmupOperations for RecordingStartupWarmupOperations {
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
    let value =
        normalize_podman_engine_ready_for_gate(Err(anyhow::anyhow!("podman binary unavailable")))
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
fn launch_snapshot_drops_expired_non_download_eta() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    let _ = job.transition_phase(
        HarnessSetupPhase::ImageLoad,
        "loading harness image into local sandbox runtime",
    );
    {
        let mut inner = lock_or_recover(&job.inner, "launch job");
        if let Some(phase) = inner.phases.last_mut() {
            phase.started_at -= chrono::TimeDelta::milliseconds(6_000);
        }
    }
    let snapshot = job.snapshot();
    assert_eq!(snapshot.current_phase, Some(HarnessSetupPhase::ImageLoad));
    assert_eq!(snapshot.eta_ms, None);
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

#[cfg(unix)]
#[tokio::test]
async fn concurrent_launch_start_is_deduplicated() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let log_path = data_dir.path().join("podman-invocations.log");
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let podman_path =
        write_running_container_podman_shim(data_dir.path(), &log_path, &container_name);
    let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let ops = Arc::new(UnexpectedRuntimeWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..Default::default()
        },
    };
    let barrier = Arc::new(Barrier::new(3));

    let coordinator_a = Arc::clone(&coordinator);
    let workspace_a = workspace.clone();
    let settings_a = settings.clone();
    let barrier_a = Arc::clone(&barrier);
    let start_a = tokio::spawn(async move {
        barrier_a.wait().await;
        coordinator_a
            .start_workspace_launch(workspace_a, settings_a, "http://127.0.0.1:4399".to_string())
            .await
    });

    let coordinator_b = Arc::clone(&coordinator);
    let workspace_b = workspace.clone();
    let settings_b = settings.clone();
    let barrier_b = Arc::clone(&barrier);
    let start_b = tokio::spawn(async move {
        barrier_b.wait().await;
        coordinator_b
            .start_workspace_launch(workspace_b, settings_b, "http://127.0.0.1:4399".to_string())
            .await
    });

    barrier.wait().await;
    let first = start_a.await.expect("first launch task failed");
    let second = start_b.await.expect("second launch task failed");
    assert_eq!(first.job_id, second.job_id);
    let workspace_id = workspace.id.0.to_string();
    assert_eq!(first.workspace_id, workspace_id);
    assert_eq!(second.workspace_id, workspace_id);

    TrackedExecutionLaunch::new(&coordinator, first.clone())
        .wait_ready(Duration::from_secs(10))
        .await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
    let exists_line = format!("container exists {container_name}");
    let inspect_line =
        format!("container inspect --format {{{{.State.Running}}}} {container_name}");
    assert_eq!(
        log.matches(&exists_line).count(),
        1,
        "expected exactly one existing-container check in log:\n{log}"
    );
    assert_eq!(
        log.matches(&inspect_line).count(),
        1,
        "expected exactly one running-container inspect in log:\n{log}"
    );
    assert!(
        !log.contains("image exists"),
        "deduplicated running-container launch should not front-load image checks:\n{log}"
    );
    assert!(
            !log.contains(&format!("start {container_name}")),
            "deduplicated running-container launch should not restart an already running container:\n{log}"
        );
    assert!(
        !log.contains("run -d --name"),
        "deduplicated running-container launch should not create a new container:\n{log}"
    );
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

    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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
    assert!(!ready.machine_ready);
    assert!(!ready.image_present);
}

#[tokio::test]
async fn runtime_prewarm_reuses_background_all_job_and_waits_for_builder_tail_when_runtime_joins() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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

    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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

    let ready = tokio::time::timeout(Duration::from_secs(10), async {
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
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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
    let _serial = env_var_test_lock().lock().await;
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

    let _terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, Duration::from_secs(5))
            .await;
}

#[tokio::test]
async fn builder_only_prewarm_skips_runtime_warmup_and_runtime_availability() {
    let _serial = env_var_test_lock().lock().await;
    let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "0");
    let data_dir = tempfile::tempdir().expect("tempdir");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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
async fn startup_prewarm_enters_shared_runtime_warmup_when_machine_is_not_ready() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let podman_path = write_startup_prewarm_podman_shim(data_dir.path());
    let _test_podman = EnvVarGuard::unset("CTX_TEST_PODMAN_AVAILABLE");
    let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    init_settings_store(data_dir.path()).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

    coordinator.run_startup_prewarm().await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(snapshot.needs_prewarm);
    assert!(!snapshot.machine_ready);
    assert!(!snapshot.image_present);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
        vec!["runtime"]
    );
}

#[tokio::test]
async fn workspace_launch_is_not_blocked_by_background_runtime_prewarm_job() {
    let _serial = env_var_test_lock().lock().await;
    let _podman = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "1");
    let data_dir = tempfile::tempdir().expect("tempdir");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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

    let background_terminal = wait_for_execution_launch_terminal(
        &coordinator,
        &background.job_id,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(background_terminal.state, ExecutionLaunchState::Ready);
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_reuses_running_container_without_runtime_prewarm_or_image_checks() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let log_path = data_dir.path().join("podman-invocations.log");
    let ops = Arc::new(UnexpectedRuntimeWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
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

    let podman_path =
        write_running_container_podman_shim(data_dir.path(), &log_path, &container_name);
    let _podman = EnvVarGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let launch = TrackedExecutionLaunch::new(
        &coordinator,
        coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await,
    );

    let ready = launch.wait_ready(Duration::from_secs(5)).await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);
    assert!(
        ready.phases.iter().all(|phase| {
            phase.phase != HarnessSetupPhase::ImageCheck
                && phase.phase != HarnessSetupPhase::ImageLoad
        }),
        "workspace launch should not emit image phases for reusable containers: {:?}",
        ready.phases
    );
    assert!(
        ready.logs.iter().all(|line| {
            line.phase != HarnessSetupPhase::ImageCheck
                && line.phase != HarnessSetupPhase::ImageLoad
        }),
        "workspace launch should not emit image logs for reusable containers: {:?}",
        ready.logs
    );

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
            !log.contains("load -i") && !log.contains("pull "),
            "workspace launch should not materialize or pull a new image for reusable containers:\n{log}"
        );
    assert!(
        !log.contains(&format!("start {container_name}")),
        "workspace launch should not restart an already running container:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "workspace launch should not create a new container when reuse is possible:\n{log}"
    );
}

#[tokio::test]
async fn workspace_launch_emits_initial_log_before_runtime_work_completes() {
    let _serial = env_var_test_lock().lock().await;
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

    let _terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, Duration::from_secs(5))
            .await;
}
