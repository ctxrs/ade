use super::super::super::*;

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
    ProviderRuntime {
        adapters: Mutex::new(providers),
        target_adapters: Mutex::new(HashMap::new()),
        statuses: Mutex::new(HashMap::new()),
        matrix_cache: Mutex::new(ctx_provider_matrix::ProviderMatrixCache::default()),
        options_cache: Mutex::new(HashMap::new()),
        verify_cache: Mutex::new(HashMap::new()),
        guard: Mutex::new(provider_guard::ProviderGuardRuntime::default()),
        restart: Mutex::new(provider_restart::ProviderRestartRuntime::default()),
        usage_cache: Mutex::new(HashMap::new()),
        codex_login_sessions: Mutex::new(HashMap::new()),
        claude_login_sessions: Mutex::new(HashMap::new()),
        gemini_login_sessions: Mutex::new(HashMap::new()),
        qwen_login_sessions: Mutex::new(HashMap::new()),
        kimi_login_sessions: Mutex::new(HashMap::new()),
        cursor_login_sessions: Mutex::new(HashMap::new()),
        amp_login_sessions: Mutex::new(HashMap::new()),
        mistral_login_sessions: Mutex::new(HashMap::new()),
        install_start_gate: Mutex::new(()),
        installs: Mutex::new(HashMap::new()),
    }
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
