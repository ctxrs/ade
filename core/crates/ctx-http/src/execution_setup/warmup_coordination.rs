use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::ids::WorkspaceId;
use tokio::sync::{broadcast, watch, Mutex};

use crate::harness_runtime::{self, HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase};
use crate::settings::ExecutionSettings;

use super::{
    seed_runtime_prewarm_initial_state, ExecutionLaunchSnapshot, ExecutionLaunchState,
    ExecutionSetupJobKind, LaunchJob, LaunchTerminalMutation, RuntimePrewarmScope,
};

const SHARED_WARMUP_EVENT_CAP: usize = 256;
const SHARED_WARMUP_CHANNEL_CAP: usize = 256;

#[async_trait]
pub(crate) trait SharedWarmupOperations: Send + Sync {
    async fn prefetch_runtime_artifacts(
        &self,
        settings: ExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()>;
}

#[derive(Clone)]
pub(crate) struct DefaultWarmupOperations {
    data_root: PathBuf,
}

impl DefaultWarmupOperations {
    pub(crate) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }
}

#[async_trait]
impl SharedWarmupOperations for DefaultWarmupOperations {
    async fn prefetch_runtime_artifacts(
        &self,
        settings: ExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let image = harness_runtime::resolve_container_image(&settings.container);
        harness_runtime::prefetch_container_startup_artifacts_with_overrides(
            &self.data_root,
            &image,
            None,
            observer,
        )
        .await
    }

    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let image = harness_runtime::resolve_container_image(&settings.container);
        harness_runtime::prefetch_container_image_with_observer(
            &self.data_root,
            &image,
            Some(observer.as_ref()),
        )
        .await
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming container builder");
        crate::container_builder::ensure_builder_ready(&self.data_root).await
    }
}

#[derive(Clone)]
pub(crate) struct LaunchPrewarmCoordinator {
    inner: Arc<LaunchPrewarmCoordinatorInner>,
}

impl LaunchPrewarmCoordinator {
    pub(crate) fn new(operations: Arc<dyn SharedWarmupOperations>) -> Self {
        Self {
            inner: Arc::new(LaunchPrewarmCoordinatorInner {
                operations,
                tasks: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub(crate) async fn ensure_scope(
        &self,
        settings: &ExecutionSettings,
        scope: RuntimePrewarmScope,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if scope.includes_runtime() {
            self.ensure_runtime(settings, observer).await?;
        }
        if scope.includes_builder() {
            self.ensure_builder(observer).await?;
        }
        Ok(())
    }

    pub(crate) async fn prefetch_runtime_artifacts(
        &self,
        settings: &ExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.inner
            .operations
            .prefetch_runtime_artifacts(settings.clone(), observer)
            .await
    }

    pub(crate) async fn ensure_runtime(
        &self,
        settings: &ExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let key = SharedWarmupKey::Runtime {
            image: harness_runtime::resolve_container_image(&settings.container),
        };
        let task = self.runtime_task(key, settings.clone()).await;
        task.attach(observer).await
    }

    pub(crate) async fn ensure_builder(
        &self,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let task = self.builder_task().await;
        task.attach(observer).await
    }

    async fn runtime_task(
        &self,
        key: SharedWarmupKey,
        settings: ExecutionSettings,
    ) -> Arc<SharedWarmupTask> {
        {
            let tasks = self.inner.tasks.lock().await;
            if let Some(existing) = tasks.get(&key) {
                return Arc::clone(existing);
            }
        }

        let task = Arc::new(SharedWarmupTask::new());
        let mut tasks = self.inner.tasks.lock().await;
        if let Some(existing) = tasks.get(&key) {
            return Arc::clone(existing);
        }
        tasks.insert(key.clone(), Arc::clone(&task));
        self.spawn_runtime_task(Arc::clone(&task), key, settings);
        task
    }

    async fn builder_task(&self) -> Arc<SharedWarmupTask> {
        let key = SharedWarmupKey::Builder;
        {
            let tasks = self.inner.tasks.lock().await;
            if let Some(existing) = tasks.get(&key) {
                return Arc::clone(existing);
            }
        }

        let task = Arc::new(SharedWarmupTask::new());
        let mut tasks = self.inner.tasks.lock().await;
        if let Some(existing) = tasks.get(&key) {
            return Arc::clone(existing);
        }
        tasks.insert(key.clone(), Arc::clone(&task));
        self.spawn_builder_task(Arc::clone(&task), key);
        task
    }

    fn spawn_runtime_task(
        &self,
        task: Arc<SharedWarmupTask>,
        key: SharedWarmupKey,
        settings: ExecutionSettings,
    ) {
        let coordinator = self.clone();
        tokio::spawn(async move {
            let observer: Arc<dyn HarnessSetupObserver> =
                Arc::new(SharedWarmupObserver::new(Arc::clone(&task)));
            let result = coordinator
                .inner
                .operations
                .warm_runtime(settings, observer)
                .await
                .map_err(|err| super::format_error_chain(&err));
            task.finish(result);
            coordinator.remove_task_if_current(&key, &task).await;
        });
    }

    fn spawn_builder_task(&self, task: Arc<SharedWarmupTask>, key: SharedWarmupKey) {
        let coordinator = self.clone();
        tokio::spawn(async move {
            let observer: Arc<dyn HarnessSetupObserver> =
                Arc::new(SharedWarmupObserver::new(Arc::clone(&task)));
            let result = coordinator
                .inner
                .operations
                .warm_builder(observer)
                .await
                .map_err(|err| super::format_error_chain(&err));
            task.finish(result);
            coordinator.remove_task_if_current(&key, &task).await;
        });
    }

    async fn remove_task_if_current(&self, key: &SharedWarmupKey, task: &Arc<SharedWarmupTask>) {
        let mut tasks = self.inner.tasks.lock().await;
        if tasks
            .get(key)
            .map(|current| Arc::ptr_eq(current, task))
            .unwrap_or(false)
        {
            tasks.remove(key);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PrewarmLaunchJobKey {
    Runtime { image: String },
    All { image: String },
    Builder,
}

impl PrewarmLaunchJobKey {
    pub(crate) fn for_request(settings: &ExecutionSettings, scope: RuntimePrewarmScope) -> Self {
        match scope {
            RuntimePrewarmScope::Runtime => Self::Runtime {
                image: harness_runtime::resolve_container_image(&settings.container),
            },
            RuntimePrewarmScope::All => Self::All {
                image: harness_runtime::resolve_container_image(&settings.container),
            },
            RuntimePrewarmScope::Builder => Self::Builder,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PrewarmJobRegistry {
    running: HashMap<PrewarmLaunchJobKey, Arc<SharedPrewarmLaunchJob>>,
}

impl PrewarmJobRegistry {
    pub(crate) fn find_compatible(
        &self,
        settings: &ExecutionSettings,
        requested_scope: RuntimePrewarmScope,
    ) -> Option<Arc<SharedPrewarmLaunchJob>> {
        match requested_scope {
            RuntimePrewarmScope::Runtime => {
                let image = harness_runtime::resolve_container_image(&settings.container);
                self.running
                    .get(&PrewarmLaunchJobKey::All {
                        image: image.clone(),
                    })
                    .cloned()
                    .or_else(|| {
                        self.running
                            .get(&PrewarmLaunchJobKey::Runtime { image })
                            .cloned()
                    })
            }
            RuntimePrewarmScope::All => self
                .running
                .get(&PrewarmLaunchJobKey::All {
                    image: harness_runtime::resolve_container_image(&settings.container),
                })
                .cloned(),
            RuntimePrewarmScope::Builder => self
                .running
                .get(&PrewarmLaunchJobKey::Builder)
                .cloned()
                .or_else(|| {
                    self.running
                        .get(&PrewarmLaunchJobKey::All {
                            image: harness_runtime::resolve_container_image(&settings.container),
                        })
                        .cloned()
                }),
        }
    }

    pub(crate) fn insert(&mut self, job: Arc<SharedPrewarmLaunchJob>) {
        self.running.insert(job.key().clone(), job);
    }

    pub(crate) fn remove_if_current(
        &mut self,
        key: &PrewarmLaunchJobKey,
        job: &Arc<SharedPrewarmLaunchJob>,
    ) {
        if self
            .running
            .get(key)
            .map(|current| Arc::ptr_eq(current, job))
            .unwrap_or(false)
        {
            self.running.remove(key);
        }
    }

    pub(crate) fn contains_job_id(&self, job_id: &str) -> bool {
        self.running
            .values()
            .any(|job| job.job().job_id.as_str() == job_id)
    }
}

#[derive(Debug)]
struct SharedPrewarmLaunchJobState {
    terminal: bool,
}

#[derive(Debug)]
pub(crate) struct SharedPrewarmLaunchJob {
    key: PrewarmLaunchJobKey,
    scope: RuntimePrewarmScope,
    job: Arc<LaunchJob>,
    state: StdMutex<SharedPrewarmLaunchJobState>,
}

impl SharedPrewarmLaunchJob {
    pub(crate) fn new(
        job_id: String,
        settings: &ExecutionSettings,
        scope: RuntimePrewarmScope,
    ) -> Self {
        let job = Arc::new(LaunchJob::new_with_kind(
            job_id,
            WorkspaceId(uuid::Uuid::nil()),
            ExecutionSetupJobKind::StartupPrewarm,
        ));
        seed_runtime_prewarm_initial_state(job.as_ref(), settings);
        Self {
            key: PrewarmLaunchJobKey::for_request(settings, scope),
            scope,
            job,
            state: StdMutex::new(SharedPrewarmLaunchJobState { terminal: false }),
        }
    }

    pub(crate) fn key(&self) -> &PrewarmLaunchJobKey {
        &self.key
    }

    pub(crate) fn job(&self) -> Arc<LaunchJob> {
        Arc::clone(&self.job)
    }

    pub(crate) fn snapshot(&self) -> ExecutionLaunchSnapshot {
        self.job.snapshot()
    }

    pub(crate) fn builder_requested(&self) -> bool {
        self.scope.includes_builder()
    }

    pub(crate) fn runtime_requested(&self) -> bool {
        self.scope.includes_runtime()
    }

    pub(crate) fn complete_ready(&self) -> Option<LaunchTerminalMutation> {
        self.complete(ExecutionLaunchState::Ready, None)
    }

    pub(crate) fn complete_error(&self, message: String) -> Option<LaunchTerminalMutation> {
        self.complete(ExecutionLaunchState::Error, Some(message))
    }

    fn complete(
        &self,
        state: ExecutionLaunchState,
        error: Option<String>,
    ) -> Option<LaunchTerminalMutation> {
        let should_complete = {
            let mut shared = match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if shared.terminal {
                false
            } else {
                shared.terminal = true;
                true
            }
        };
        if should_complete {
            Some(self.job.mark_terminal(state, error))
        } else {
            None
        }
    }
}

struct LaunchPrewarmCoordinatorInner {
    operations: Arc<dyn SharedWarmupOperations>,
    tasks: Mutex<HashMap<SharedWarmupKey, Arc<SharedWarmupTask>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SharedWarmupKey {
    Runtime { image: String },
    Builder,
}

#[derive(Debug, Clone)]
enum SharedWarmupEventKind {
    Phase {
        phase: HarnessSetupPhase,
        message: String,
    },
    Log {
        phase: HarnessSetupPhase,
        level: HarnessSetupLogLevel,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct SharedWarmupEvent {
    seq: u64,
    kind: SharedWarmupEventKind,
}

impl SharedWarmupEvent {
    fn emit(&self, observer: &dyn HarnessSetupObserver) {
        match &self.kind {
            SharedWarmupEventKind::Phase { phase, message } => observer.on_phase(*phase, message),
            SharedWarmupEventKind::Log {
                phase,
                level,
                message,
            } => observer.on_log(*phase, *level, message),
        }
    }
}

#[derive(Debug, Clone)]
enum SharedWarmupStatus {
    Running,
    Ready,
    Error(String),
}

struct SharedWarmupTaskState {
    events: VecDeque<SharedWarmupEvent>,
    next_seq: u64,
}

struct SharedWarmupTask {
    inner: StdMutex<SharedWarmupTaskState>,
    tx: broadcast::Sender<SharedWarmupEvent>,
    status_tx: watch::Sender<SharedWarmupStatus>,
}

impl SharedWarmupTask {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(SHARED_WARMUP_CHANNEL_CAP);
        let (status_tx, _) = watch::channel(SharedWarmupStatus::Running);
        Self {
            inner: StdMutex::new(SharedWarmupTaskState {
                events: VecDeque::new(),
                next_seq: 0,
            }),
            tx,
            status_tx,
        }
    }

    async fn attach(&self, observer: Option<&dyn HarnessSetupObserver>) -> Result<()> {
        let mut rx = self.tx.subscribe();
        let mut status_rx = self.status_tx.subscribe();
        let (events, mut last_seq, initial_status) = {
            let inner = match self.inner.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let events = inner.events.iter().cloned().collect::<Vec<_>>();
            let last_seq = events.last().map(|event| event.seq).unwrap_or(0);
            (events, last_seq, status_rx.borrow().clone())
        };

        if let Some(observer) = observer {
            for event in &events {
                event.emit(observer);
            }
        }

        match initial_status {
            SharedWarmupStatus::Running => {}
            SharedWarmupStatus::Ready => return Ok(()),
            SharedWarmupStatus::Error(message) => return Err(anyhow::anyhow!(message)),
        }

        loop {
            tokio::select! {
                recv = rx.recv() => {
                    match recv {
                        Ok(event) => {
                            if event.seq <= last_seq {
                                continue;
                            }
                            if let Some(observer) = observer {
                                event.emit(observer);
                            }
                            last_seq = event.seq;
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            match status_rx.borrow().clone() {
                                SharedWarmupStatus::Running => continue,
                                SharedWarmupStatus::Ready => return Ok(()),
                                SharedWarmupStatus::Error(message) => {
                                    return Err(anyhow::anyhow!(message));
                                }
                            }
                        }
                    }
                }
                changed = status_rx.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    match status_rx.borrow().clone() {
                        SharedWarmupStatus::Running => {}
                        SharedWarmupStatus::Ready => return Ok(()),
                        SharedWarmupStatus::Error(message) => return Err(anyhow::anyhow!(message)),
                    }
                }
            }
        }
    }

    fn finish(&self, result: std::result::Result<(), String>) {
        let status = match result {
            Ok(()) => SharedWarmupStatus::Ready,
            Err(message) => SharedWarmupStatus::Error(message),
        };
        let _ = self.status_tx.send(status);
    }

    fn push_event(&self, kind: SharedWarmupEventKind) {
        let event = {
            let mut inner = match self.inner.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            inner.next_seq += 1;
            let event = SharedWarmupEvent {
                seq: inner.next_seq,
                kind,
            };
            inner.events.push_back(event.clone());
            while inner.events.len() > SHARED_WARMUP_EVENT_CAP {
                inner.events.pop_front();
            }
            event
        };
        let _ = self.tx.send(event);
    }
}

struct SharedWarmupObserver {
    task: Arc<SharedWarmupTask>,
}

impl SharedWarmupObserver {
    fn new(task: Arc<SharedWarmupTask>) -> Self {
        Self { task }
    }
}

impl HarnessSetupObserver for SharedWarmupObserver {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str) {
        self.task.push_event(SharedWarmupEventKind::Phase {
            phase,
            message: message.to_string(),
        });
    }

    fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str) {
        self.task.push_event(SharedWarmupEventKind::Log {
            phase,
            level,
            message: message.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::*;
    use crate::settings::{ExecutionMode, ExecutionSettings};

    #[derive(Default)]
    struct FakeWarmupOperations {
        runtime_runs: AtomicUsize,
        builder_runs: AtomicUsize,
        runtime_release: Notify,
        builder_release: Notify,
        runtime_block: bool,
        builder_block: bool,
    }

    impl FakeWarmupOperations {
        fn blocking_runtime() -> Self {
            Self {
                runtime_block: true,
                ..Self::default()
            }
        }

        fn blocking_runtime_and_builder() -> Self {
            Self {
                runtime_block: true,
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

        fn release_runtime(&self) {
            self.runtime_release.notify_waiters();
        }

        fn release_builder(&self) {
            self.builder_release.notify_waiters();
        }
    }

    #[async_trait]
    impl SharedWarmupOperations for FakeWarmupOperations {
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
            if self.runtime_block {
                self.runtime_release.notified().await;
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
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };
        settings.container.image = Some(image.to_string());
        settings
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
        assert_eq!(ops.builder_runs.load(Ordering::SeqCst), 0);
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

        ops.wait_for_runtime_runs(1).await;

        let foreground_coordinator = coordinator.clone();
        let foreground_settings = settings.clone();
        let foreground = tokio::spawn(async move {
            foreground_coordinator
                .ensure_scope(&foreground_settings, RuntimePrewarmScope::Runtime, None)
                .await
        });

        ops.expect_runtime_runs_below(2).await;

        ops.release_runtime();
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
}
