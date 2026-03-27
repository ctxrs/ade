use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::Notify;

use super::*;
use crate::settings::{ExecutionMode, ExecutionSettings};

#[derive(Default)]
struct FakeWarmupOperations {
    runtime_runs: AtomicUsize,
    launch_ready_runs: AtomicUsize,
    builder_runs: AtomicUsize,
    runtime_release: Notify,
    launch_ready_release: Notify,
    builder_release: Notify,
    runtime_block: bool,
    launch_ready_block: bool,
    builder_block: bool,
}

impl FakeWarmupOperations {
    fn blocking_runtime() -> Self {
        Self {
            runtime_block: true,
            ..Self::default()
        }
    }

    fn blocking_launch_ready() -> Self {
        Self {
            launch_ready_block: true,
            ..Self::default()
        }
    }

    fn blocking_runtime_and_builder() -> Self {
        Self {
            launch_ready_block: true,
            builder_block: true,
            ..Self::default()
        }
    }

    async fn wait_for_runtime_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
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

    async fn expect_runtime_runs_below(&self, unexpected: usize) {
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                self.wait_for_runtime_runs(unexpected)
            )
            .await
            .is_err(),
            "unexpectedly observed runtime run {unexpected}",
        );
    }

    async fn wait_for_builder_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
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

    async fn wait_for_launch_ready_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if self.launch_ready_runs.load(Ordering::SeqCst) >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for launch-ready runs");
    }

    fn release_runtime(&self) {
        self.runtime_release.notify_waiters();
    }

    fn release_launch_ready(&self) {
        self.launch_ready_release.notify_waiters();
    }

    fn release_builder(&self) {
        self.builder_release.notify_waiters();
    }
}

#[async_trait]
impl SharedWarmupOperations for FakeWarmupOperations {
    async fn warm_runtime(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        observer.on_phase(HarnessSetupPhase::MachineCheck, "warming runtime");
        observer.on_progress(HarnessSetupProgressUpdate {
            phase: HarnessSetupPhase::ArtifactDownload,
            active_download: Some(crate::harness_runtime::HarnessSetupDownloadStatus {
                artifact: "Required artifacts".to_string(),
                downloaded_bytes: 512,
                total_bytes: Some(1024),
                bytes_per_sec: Some(128),
            }),
        });
        if self.runtime_block {
            self.runtime_release.notified().await;
        }
        Ok(())
    }

    async fn warm_runtime_launch_ready(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.launch_ready_runs.fetch_add(1, Ordering::SeqCst);
        observer.on_phase(
            HarnessSetupPhase::MachineStartOrInit,
            "warming launch-ready runtime",
        );
        if self.launch_ready_block {
            self.launch_ready_release.notified().await;
        }
        Ok(())
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        self.builder_runs.fetch_add(1, Ordering::SeqCst);
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming builder");
        if self.builder_block {
            self.builder_release.notified().await;
        }
        Ok(())
    }
}

fn container_settings(image: &str) -> ExecutionSettings {
    let mut settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        ..ExecutionSettings::default()
    };
    settings.container.image = Some(image.to_string());
    settings
}

#[derive(Default)]
struct RecordingObserver {
    phases: StdMutex<Vec<(HarnessSetupPhase, String)>>,
    progress: StdMutex<Vec<HarnessSetupProgressUpdate>>,
}

impl HarnessSetupObserver for RecordingObserver {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str) {
        self.phases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((phase, message.to_string()));
    }

    fn on_log(&self, _phase: HarnessSetupPhase, _level: HarnessSetupLogLevel, _message: &str) {}

    fn on_progress(&self, progress: HarnessSetupProgressUpdate) {
        self.progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(progress);
    }
}

#[tokio::test]
async fn shared_runtime_warmup_is_deduplicated_for_same_image() {
    let ops = Arc::new(FakeWarmupOperations::blocking_runtime());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");

    let coord_a = coordinator.clone();
    let settings_a = settings.clone();
    let first = tokio::spawn(async move {
        coord_a
            .ensure_scope(&settings_a, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.wait_for_runtime_runs(1).await;

    let coord_b = coordinator.clone();
    let settings_b = settings.clone();
    let second = tokio::spawn(async move {
        coord_b
            .ensure_scope(&settings_b, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.expect_runtime_runs_below(2).await;

    ops.release_runtime();

    first
        .await
        .expect("first runtime wait failed")
        .expect("first runtime wait errored");
    second
        .await
        .expect("second runtime wait failed")
        .expect("second runtime wait errored");
}

#[tokio::test]
async fn runtime_warmup_does_not_deduplicate_different_images() {
    let ops = Arc::new(FakeWarmupOperations::blocking_runtime());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings_a = container_settings("ghcr.io/ctxrs/ctx-harness:test-a");
    let settings_b = container_settings("ghcr.io/ctxrs/ctx-harness:test-b");

    let coord_a = coordinator.clone();
    let first = tokio::spawn(async move {
        coord_a
            .ensure_scope(&settings_a, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.wait_for_runtime_runs(1).await;

    let coord_b = coordinator.clone();
    let second = tokio::spawn(async move {
        coord_b
            .ensure_scope(&settings_b, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.wait_for_runtime_runs(2).await;
    ops.release_runtime();

    first
        .await
        .expect("first runtime wait failed")
        .expect("first runtime wait errored");
    second
        .await
        .expect("second runtime wait failed")
        .expect("second runtime wait errored");

    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn runtime_scope_does_not_invoke_builder_warmup() {
    let ops = Arc::new(FakeWarmupOperations::default());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");

    coordinator
        .ensure_scope(&settings, RuntimePrewarmScope::Runtime, None)
        .await
        .expect("runtime scope should succeed");

    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(ops.launch_ready_runs.load(Ordering::SeqCst), 0);
    assert_eq!(ops.builder_runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn late_runtime_joiner_replays_progress_updates() {
    let ops = Arc::new(FakeWarmupOperations::blocking_runtime());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");
    let first_observer = Arc::new(RecordingObserver::default());

    let background_coordinator = coordinator.clone();
    let background_settings = settings.clone();
    let background_observer = first_observer.clone();
    let first = tokio::spawn(async move {
        background_coordinator
            .ensure_scope(
                &background_settings,
                RuntimePrewarmScope::Runtime,
                Some(background_observer.as_ref()),
            )
            .await
    });

    ops.wait_for_runtime_runs(1).await;

    let second_observer = Arc::new(RecordingObserver::default());
    let join_coordinator = coordinator.clone();
    let join_settings = settings.clone();
    let join_observer = second_observer.clone();
    let second = tokio::spawn(async move {
        join_coordinator
            .ensure_scope(
                &join_settings,
                RuntimePrewarmScope::Runtime,
                Some(join_observer.as_ref()),
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !second_observer
                .progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for replayed progress");

    let replayed = second_observer
        .progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert!(replayed.iter().any(|update| {
        update.phase == HarnessSetupPhase::ArtifactDownload
            && update
                .active_download
                .as_ref()
                .map(|download| download.downloaded_bytes == 512)
                .unwrap_or(false)
    }));

    ops.release_runtime();

    first
        .await
        .expect("first runtime wait failed")
        .expect("first runtime wait errored");
    second
        .await
        .expect("second runtime wait failed")
        .expect("second runtime wait errored");
}

#[tokio::test]
async fn foreground_runtime_request_finishes_before_background_all_builder_work() {
    let ops = Arc::new(FakeWarmupOperations::blocking_runtime_and_builder());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");

    let background_coordinator = coordinator.clone();
    let background_settings = settings.clone();
    let background = tokio::spawn(async move {
        background_coordinator
            .ensure_scope(&background_settings, RuntimePrewarmScope::All, None)
            .await
    });

    ops.wait_for_launch_ready_runs(1).await;

    let foreground_coordinator = coordinator.clone();
    let foreground_settings = settings.clone();
    let foreground = tokio::spawn(async move {
        foreground_coordinator
            .ensure_scope(&foreground_settings, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.expect_runtime_runs_below(2).await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    ops.release_launch_ready();
    ops.wait_for_builder_runs(1).await;

    tokio::time::timeout(Duration::from_secs(1), foreground)
        .await
        .expect("foreground runtime wait timed out")
        .expect("foreground runtime task failed")
        .expect("foreground runtime wait errored");

    assert!(!background.is_finished());

    ops.release_builder();

    background
        .await
        .expect("background all wait failed")
        .expect("background all wait errored");
}

#[test]
fn prewarm_job_registry_matches_compatible_jobs_by_scope_and_image() {
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");
    let other_settings = container_settings("ghcr.io/ctxrs/ctx-harness:other");

    let all_job = Arc::new(SharedPrewarmLaunchJob::new(
        "job-all".to_string(),
        &settings,
        RuntimePrewarmScope::All,
    ));
    let builder_job = Arc::new(SharedPrewarmLaunchJob::new(
        "job-builder".to_string(),
        &settings,
        RuntimePrewarmScope::Builder,
    ));
    let other_runtime_job = Arc::new(SharedPrewarmLaunchJob::new(
        "job-other-runtime".to_string(),
        &other_settings,
        RuntimePrewarmScope::Runtime,
    ));

    let mut registry = PrewarmJobRegistry::default();
    registry.insert(Arc::clone(&all_job));
    registry.insert(Arc::clone(&builder_job));
    registry.insert(Arc::clone(&other_runtime_job));

    let runtime_match = registry
        .find_compatible(&settings, RuntimePrewarmScope::Runtime)
        .expect("runtime request should reuse matching all-scope job");
    assert!(Arc::ptr_eq(&runtime_match, &all_job));

    let builder_match = registry
        .find_compatible(&settings, RuntimePrewarmScope::Builder)
        .expect("builder request should prefer exact builder job");
    assert!(Arc::ptr_eq(&builder_match, &builder_job));

    let launch_ready_match = registry
        .find_compatible(&settings, RuntimePrewarmScope::LaunchReady)
        .expect("launch-ready request should reuse matching all-scope job");
    assert!(Arc::ptr_eq(&launch_ready_match, &all_job));

    let all_match = registry
        .find_compatible(&settings, RuntimePrewarmScope::All)
        .expect("all-scope request should match all-scope job");
    assert!(Arc::ptr_eq(&all_match, &all_job));

    assert!(
        registry
            .find_compatible(&other_settings, RuntimePrewarmScope::All)
            .is_none(),
        "runtime-only job for another image should not satisfy all-scope requests"
    );
}

#[test]
fn prewarm_job_registry_remove_if_current_only_removes_exact_pointer() {
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");
    let stale_job = Arc::new(SharedPrewarmLaunchJob::new(
        "job-stale".to_string(),
        &settings,
        RuntimePrewarmScope::Runtime,
    ));
    let current_job = Arc::new(SharedPrewarmLaunchJob::new(
        "job-current".to_string(),
        &settings,
        RuntimePrewarmScope::Runtime,
    ));
    let key = PrewarmLaunchJobKey::for_request(&settings, RuntimePrewarmScope::Runtime);

    let mut registry = PrewarmJobRegistry::default();
    registry.insert(Arc::clone(&stale_job));
    registry.insert(Arc::clone(&current_job));

    registry.remove_if_current(&key, &stale_job);
    let still_present = registry
        .find_compatible(&settings, RuntimePrewarmScope::Runtime)
        .expect("current job should remain after stale-pointer removal");
    assert!(Arc::ptr_eq(&still_present, &current_job));

    registry.remove_if_current(&key, &current_job);
    assert!(
        registry
            .find_compatible(&settings, RuntimePrewarmScope::Runtime)
            .is_none(),
        "current pointer removal should clear the registry entry"
    );
}

#[test]
fn shared_prewarm_launch_job_terminal_completion_is_idempotent() {
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");
    let job = SharedPrewarmLaunchJob::new(
        "job-terminal".to_string(),
        &settings,
        RuntimePrewarmScope::Runtime,
    );

    assert!(job.runtime_requested());
    assert!(!job.requires_launch_ready_runtime());
    assert!(!job.builder_requested());
    assert!(job.complete_ready().is_some());

    let snapshot = job.snapshot();
    assert_eq!(snapshot.state, ExecutionLaunchState::Ready);
    assert!(snapshot.error.is_none());

    assert!(
        job.complete_error("should not apply".to_string()).is_none(),
        "second terminal completion should be ignored"
    );

    let snapshot_after = job.snapshot();
    assert_eq!(snapshot_after.state, ExecutionLaunchState::Ready);
    assert!(snapshot_after.error.is_none());
}

#[tokio::test]
async fn runtime_scope_reuses_running_launch_ready_task_for_same_target() {
    let ops = Arc::new(FakeWarmupOperations::blocking_launch_ready());
    let coordinator = LaunchPrewarmCoordinator::new(ops.clone());
    let settings = container_settings("ghcr.io/ctxrs/ctx-harness:test");

    let background_coordinator = coordinator.clone();
    let background_settings = settings.clone();
    let background = tokio::spawn(async move {
        background_coordinator
            .ensure_scope(&background_settings, RuntimePrewarmScope::LaunchReady, None)
            .await
    });

    ops.wait_for_launch_ready_runs(1).await;

    let foreground_coordinator = coordinator.clone();
    let foreground_settings = settings.clone();
    let foreground = tokio::spawn(async move {
        foreground_coordinator
            .ensure_scope(&foreground_settings, RuntimePrewarmScope::Runtime, None)
            .await
    });

    ops.expect_runtime_runs_below(1).await;
    ops.release_launch_ready();

    foreground
        .await
        .expect("foreground runtime wait failed")
        .expect("foreground runtime wait errored");
    background
        .await
        .expect("background launch-ready wait failed")
        .expect("background launch-ready wait errored");

    assert_eq!(ops.launch_ready_runs.load(Ordering::SeqCst), 1);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);
}
