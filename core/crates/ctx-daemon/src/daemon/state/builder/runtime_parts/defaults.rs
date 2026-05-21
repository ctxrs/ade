use super::super::super::*;
use ctx_worktree_vcs_service::{
    worktree_vcs_scheduler_concurrency_from_env, WorktreeVcsSchedulerRuntime,
};

pub(in crate::daemon::state::builder) fn build_workspace_runtime(
    worktree_vcs_enabled: bool,
    workspace_active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
) -> WorkspaceRuntime {
    WorkspaceRuntime {
        worktree_vcs_enabled,
        file_completions_cache: Mutex::new(HashMap::new()),
        workspace_file_completions_cache: Mutex::new(HashMap::new()),
        git_status_snapshots: Mutex::new(HashMap::new()),
        worktree_vcs_snapshots: Mutex::new(HashMap::new()),
        worktree_vcs_active: Mutex::new(HashMap::new()),
        worktree_vcs_refresh_locks: Mutex::new(HashMap::new()),
        worktree_vcs_open_panes: Mutex::new(HashMap::new()),
        worktree_vcs_summary_gen: Mutex::new(HashMap::new()),
        worktree_vcs_runtime: Mutex::new(HashMap::new()),
        worktree_vcs_scheduler: WorktreeVcsSchedulerRuntime::with_concurrency(
            worktree_vcs_scheduler_concurrency_from_env(),
        ),
        worktree_vcs_events: broadcast::channel(1024).0,
        git_status_watchers: Mutex::new(HashSet::new()),
        workspace_active_snapshot,
        workspace_active_snapshot_cache: Mutex::new(HashMap::new()),
        workspace_active_heads_cache: Mutex::new(HashMap::new()),
        worktree_bootstrap_gates: Mutex::new(HashMap::new()),
        attachment_materializations: Mutex::new(HashMap::new()),
        attachment_materialization_generation: AtomicU64::new(0),
    }
}

pub(in crate::daemon::state::builder) fn build_provider_runtime(
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
) -> ProviderRuntime {
    ProviderRuntime::new(providers)
}

pub(in crate::daemon::state::builder) fn build_telemetry_runtime(
    telemetry: Telemetry,
    ops_events: OpsEvents,
    perf_telemetry: PerfTelemetry,
    provider_unknown_events: ctx_observability::provider_unknown_events::ProviderUnknownEvents,
) -> TelemetryRuntime {
    TelemetryRuntime {
        telemetry,
        ops_events,
        perf_telemetry,
        provider_unknown_events,
        resource_governance: Mutex::new(ResourceGovernanceRuntime::default()),
        resource_sampler: Mutex::new(ResourceSampler::new()),
    }
}

pub(in crate::daemon::state::builder) fn build_transport_runtime(
    terminals: Arc<TerminalManager>,
    web_sessions: Arc<WebSessionManager>,
) -> TransportRuntime {
    TransportRuntime {
        terminals,
        mobile_tunnel: MobileTunnelManager::default(),
        web_sessions,
        merge_queue: Arc::new(ctx_merge_queue::MergeQueueRuntime::new()),
    }
}

pub(in crate::daemon::state::builder) fn build_execution_runtime(
    harness_runtime: Arc<HarnessRuntimeManager>,
    execution_setup: Arc<ExecutionSetupCoordinator>,
) -> ExecutionRuntime {
    ExecutionRuntime {
        harness: harness_runtime,
        setup: execution_setup,
    }
}
