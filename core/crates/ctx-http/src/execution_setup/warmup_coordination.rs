use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::ids::WorkspaceId;
use tokio::sync::{broadcast, watch, Mutex};

use crate::workspace_runtime::{
    self, HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase, HarnessSetupProgressUpdate,
};
use crate::settings::ExecutionSettings;

use super::{
    seed_runtime_prewarm_initial_state, ExecutionLaunchSnapshot, ExecutionLaunchState,
    ExecutionSetupJobKind, LaunchJob, LaunchTerminalMutation, RuntimePrewarmScope,
};

const SHARED_WARMUP_EVENT_CAP: usize = 256;
const SHARED_WARMUP_CHANNEL_CAP: usize = 256;

#[async_trait]
pub(crate) trait SharedWarmupOperations: Send + Sync {
    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_runtime_launch_ready(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()>;
}

#[derive(Clone)]
pub(crate) struct DefaultWarmupOperations {
    data_root: PathBuf,
    ops_events: crate::ops_events::OpsEvents,
}

impl DefaultWarmupOperations {
    pub(crate) fn new(data_root: PathBuf, ops_events: crate::ops_events::OpsEvents) -> Self {
        Self {
            data_root,
            ops_events,
        }
    }
}

#[async_trait]
impl SharedWarmupOperations for DefaultWarmupOperations {
    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        workspace_runtime::prewarm_selected_runtime_with_observer(
            &self.data_root,
            &settings.container,
            Some(observer.as_ref()),
        )
        .await
    }

    async fn warm_runtime_launch_ready(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        workspace_runtime::prewarm_selected_runtime_for_launch_with_observer(
            &self.data_root,
            &settings.container,
            Some(observer.as_ref()),
        )
        .await?;
        if let Some(record) = workspace_runtime::selected_shared_substrate_lifecycle(&self.data_root)?
        {
            self.ops_events
                .emit(crate::ops_events::substrate_lifecycle_observed_event(
                    &record,
                    crate::ops_events::SubstrateLifecycleOpsEventContext {
                        source: "runtime_prewarm_launch_ready",
                        workspace_id: None,
                    },
                ));
        }
        Ok(())
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming container builder");
        crate::container_builder::ensure_builder_ready(&self.data_root).await?;
        if let Some(record) = workspace_runtime::selected_shared_substrate_lifecycle(&self.data_root)?
        {
            self.ops_events
                .emit(crate::ops_events::substrate_lifecycle_observed_event(
                    &record,
                    crate::ops_events::SubstrateLifecycleOpsEventContext {
                        source: "builder_prewarm",
                        workspace_id: None,
                    },
                ));
        }
        Ok(())
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
            self.ensure_runtime(settings, scope.requires_launch_ready_runtime(), observer)
                .await?;
        }
        if scope.includes_builder() {
            self.ensure_builder(observer).await?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_runtime(
        &self,
        settings: &ExecutionSettings,
        requires_launch_ready: bool,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let task = self
            .runtime_task(settings.clone(), requires_launch_ready)
            .await;
        task.attach(observer).await
    }

    pub(crate) async fn attach_runtime_if_running(
        &self,
        settings: &ExecutionSettings,
        requires_launch_ready: bool,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        let task = self
            .runtime_task_if_running(settings, requires_launch_ready)
            .await;
        let Some(task) = task else {
            return Ok(false);
        };
        task.attach(observer).await?;
        Ok(true)
    }

    pub(crate) async fn runtime_is_running(
        &self,
        settings: &ExecutionSettings,
        requires_launch_ready: bool,
    ) -> bool {
        let tasks = self.inner.tasks.lock().await;
        runtime_task_keys(settings, requires_launch_ready)
            .iter()
            .any(|key| tasks.contains_key(key))
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
        settings: ExecutionSettings,
        requires_launch_ready: bool,
    ) -> Arc<SharedWarmupTask> {
        if let Some(existing) = self
            .runtime_task_if_running(&settings, requires_launch_ready)
            .await
        {
            return existing;
        }

        let key = exact_runtime_task_key(&settings, requires_launch_ready);
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

    async fn runtime_task_if_running(
        &self,
        settings: &ExecutionSettings,
        requires_launch_ready: bool,
    ) -> Option<Arc<SharedWarmupTask>> {
        let tasks = self.inner.tasks.lock().await;
        for key in runtime_task_keys(settings, requires_launch_ready) {
            if let Some(existing) = tasks.get(&key) {
                return Some(Arc::clone(existing));
            }
        }
        None
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
                .warm_runtime_with_key(&key, settings, observer)
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

    async fn warm_runtime_with_key(
        &self,
        key: &SharedWarmupKey,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        match key {
            SharedWarmupKey::Runtime { .. } => {
                self.inner.operations.warm_runtime(settings, observer).await
            }
            SharedWarmupKey::LaunchReady { .. } => {
                self.inner
                    .operations
                    .warm_runtime_launch_ready(settings, observer)
                    .await
            }
            SharedWarmupKey::Builder => anyhow::bail!("builder key is not a runtime warmup task"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PrewarmLaunchJobKey {
    Runtime { target: String },
    LaunchReady { target: String },
    All { target: String },
    Builder,
}

impl PrewarmLaunchJobKey {
    pub(crate) fn for_request(settings: &ExecutionSettings, scope: RuntimePrewarmScope) -> Self {
        match scope {
            RuntimePrewarmScope::Runtime => Self::Runtime {
                target: workspace_runtime::runtime_prewarm_target(&settings.container),
            },
            RuntimePrewarmScope::LaunchReady => Self::LaunchReady {
                target: workspace_runtime::runtime_prewarm_target(&settings.container),
            },
            RuntimePrewarmScope::All => Self::All {
                target: workspace_runtime::runtime_prewarm_target(&settings.container),
            },
            RuntimePrewarmScope::Builder => Self::Builder,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestedPrewarmScope {
    runtime: bool,
    launch_ready: bool,
    builder: bool,
}

impl RequestedPrewarmScope {
    fn from_runtime_prewarm_scope(scope: RuntimePrewarmScope) -> Self {
        Self {
            runtime: scope.includes_runtime(),
            launch_ready: scope.requires_launch_ready_runtime(),
            builder: scope.includes_builder(),
        }
    }

    fn merge_request(&mut self, scope: RuntimePrewarmScope) {
        let requested = Self::from_runtime_prewarm_scope(scope);
        self.runtime |= requested.runtime;
        self.launch_ready |= requested.launch_ready;
        self.builder |= requested.builder;
    }

    pub(crate) fn runtime_requested(self) -> bool {
        self.runtime
    }

    pub(crate) fn requires_launch_ready_runtime(self) -> bool {
        self.launch_ready
    }

    pub(crate) fn builder_requested(self) -> bool {
        self.builder
    }

    fn is_satisfied(self, runtime_ready: bool, launch_ready: bool, builder_ready: bool) -> bool {
        let runtime_satisfied = if self.launch_ready {
            launch_ready
        } else if self.runtime {
            runtime_ready || launch_ready
        } else {
            true
        };
        let builder_satisfied = !self.builder || builder_ready;
        runtime_satisfied && builder_satisfied
    }
}

#[derive(Debug, Default)]
pub(crate) struct PrewarmJobRegistry {
    running: HashMap<PrewarmLaunchJobKey, Arc<SharedPrewarmLaunchJob>>,
}

impl PrewarmJobRegistry {
    fn runtime_job_for_target(&self, target: &str) -> Option<Arc<SharedPrewarmLaunchJob>> {
        self.running
            .get(&PrewarmLaunchJobKey::All {
                target: target.to_string(),
            })
            .cloned()
            .or_else(|| {
                self.running
                    .get(&PrewarmLaunchJobKey::LaunchReady {
                        target: target.to_string(),
                    })
                    .cloned()
            })
            .or_else(|| {
                self.running
                    .get(&PrewarmLaunchJobKey::Runtime {
                        target: target.to_string(),
                    })
                    .cloned()
            })
    }

    pub(crate) fn find_compatible(
        &self,
        settings: &ExecutionSettings,
        requested_scope: RuntimePrewarmScope,
    ) -> Option<Arc<SharedPrewarmLaunchJob>> {
        let target = workspace_runtime::runtime_prewarm_target(&settings.container);
        match requested_scope {
            RuntimePrewarmScope::Runtime
            | RuntimePrewarmScope::LaunchReady
            | RuntimePrewarmScope::All => self.runtime_job_for_target(&target),
            RuntimePrewarmScope::Builder => self
                .running
                .get(&PrewarmLaunchJobKey::Builder)
                .cloned()
                .or_else(|| self.runtime_job_for_target(&target)),
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
    requested_scope: RequestedPrewarmScope,
}

#[derive(Debug)]
pub(crate) struct SharedPrewarmLaunchJob {
    key: PrewarmLaunchJobKey,
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
            job,
            state: StdMutex::new(SharedPrewarmLaunchJobState {
                terminal: false,
                requested_scope: RequestedPrewarmScope::from_runtime_prewarm_scope(scope),
            }),
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

    pub(crate) fn request_scope(&self, scope: RuntimePrewarmScope) -> bool {
        let mut shared = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if shared.terminal {
            return false;
        }
        shared.requested_scope.merge_request(scope);
        true
    }

    pub(crate) fn requested_scope(&self) -> RequestedPrewarmScope {
        let shared = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        shared.requested_scope
    }

    #[cfg(test)]
    pub(crate) fn builder_requested(&self) -> bool {
        self.requested_scope().builder_requested()
    }

    #[cfg(test)]
    pub(crate) fn runtime_requested(&self) -> bool {
        self.requested_scope().runtime_requested()
    }

    #[cfg(test)]
    pub(crate) fn requires_launch_ready_runtime(&self) -> bool {
        self.requested_scope().requires_launch_ready_runtime()
    }

    pub(crate) fn complete_ready(&self) -> Option<LaunchTerminalMutation> {
        self.complete(ExecutionLaunchState::Ready, None)
    }

    pub(crate) fn reserve_ready_completion_if_scope_satisfied(
        &self,
        runtime_ready: bool,
        launch_ready: bool,
        builder_ready: bool,
    ) -> Option<RequestedPrewarmScope> {
        {
            let mut shared = match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if shared.terminal
                || !shared
                    .requested_scope
                    .is_satisfied(runtime_ready, launch_ready, builder_ready)
            {
                None
            } else {
                shared.terminal = true;
                Some(shared.requested_scope)
            }
        }
    }

    pub(crate) fn mark_reserved_ready_terminal(&self) -> LaunchTerminalMutation {
        self.job.mark_terminal(ExecutionLaunchState::Ready, None)
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
    Runtime { target: String },
    LaunchReady { target: String },
    Builder,
}

fn exact_runtime_task_key(
    settings: &ExecutionSettings,
    requires_launch_ready: bool,
) -> SharedWarmupKey {
    let target = workspace_runtime::runtime_prewarm_target(&settings.container);
    if requires_launch_ready {
        SharedWarmupKey::LaunchReady { target }
    } else {
        SharedWarmupKey::Runtime { target }
    }
}

fn runtime_task_keys(
    settings: &ExecutionSettings,
    requires_launch_ready: bool,
) -> Vec<SharedWarmupKey> {
    let target = workspace_runtime::runtime_prewarm_target(&settings.container);
    if requires_launch_ready {
        vec![SharedWarmupKey::LaunchReady {
            target: target.clone(),
        }]
    } else {
        vec![
            SharedWarmupKey::LaunchReady {
                target: target.clone(),
            },
            SharedWarmupKey::Runtime { target },
        ]
    }
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
    Progress {
        progress: HarnessSetupProgressUpdate,
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
            SharedWarmupEventKind::Progress { progress } => observer.on_progress(progress.clone()),
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

    fn on_progress(&self, progress: HarnessSetupProgressUpdate) {
        self.task
            .push_event(SharedWarmupEventKind::Progress { progress });
    }
}

#[cfg(test)]
mod tests;
