use super::*;

impl AppState {
    pub fn new(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
    ) -> Self {
        Self::new_with_lsp_config(
            data_root,
            stores,
            providers,
            daemon_url,
            auth_token,
            LspManagerConfig::default(),
        )
    }

    pub fn new_with_lsp_config(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
    ) -> Self {
        let lsp_edit_plans_enabled = std::env::var("CTX_LSP_EDITPLANS_ENABLED")
            .ok()
            .as_deref()
            .and_then(ctx_core::boolish::parse_boolish)
            .unwrap_or(false);
        Self::new_with_lsp_config_and_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            auth_token,
            lsp_cfg,
            lsp_edit_plans_enabled,
        )
    }

    pub fn new_with_lsp_config_and_flags(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
        lsp_edit_plans_enabled: bool,
    ) -> Self {
        Self::new_with_lsp_config_and_runtime_flags(
            data_root,
            stores,
            providers,
            daemon_url,
            auth_token,
            lsp_cfg,
            AppRuntimeFlags {
                lsp_edit_plans_enabled,
                worktree_vcs_enabled: worktree_vcs_enabled_from_env(),
            },
        )
    }

    pub fn new_with_lsp_config_and_runtime_flags(
        data_root: PathBuf,
        stores: StoreManager,
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        daemon_url: String,
        auth_token: Option<String>,
        lsp_cfg: LspManagerConfig,
        runtime_flags: AppRuntimeFlags,
    ) -> Self {
        let lsp_edit_plans_enabled = runtime_flags.lsp_edit_plans_enabled;
        let worktree_vcs_enabled = runtime_flags.worktree_vcs_enabled;
        // Internal spool-path mechanics remain experimental, but once output is
        // promoted into the session artifact list it follows the normal
        // SessionState/artifact client contract.
        let tool_output_spool_enabled = std::env::var("CTX_TOOL_OUTPUT_DISK_SPOOL")
            .ok()
            .as_deref()
            .and_then(ctx_core::boolish::parse_boolish)
            .unwrap_or(false);
        let tool_output_spool_dir = data_root.join("tool-output-spool");
        if tool_output_spool_enabled {
            if let Err(e) = std::fs::create_dir_all(&tool_output_spool_dir) {
                tracing::warn!(
                    "failed to create tool output spool dir {}: {e}",
                    tool_output_spool_dir.to_string_lossy()
                );
            }
        }
        let edit_plans_dir = edit_plans::edit_plans_dir(&data_root);
        if let Err(e) = std::fs::create_dir_all(&edit_plans_dir) {
            tracing::warn!(
                "failed to create edit plans dir {}: {e}",
                edit_plans_dir.to_string_lossy()
            );
        }
        let edit_plans = edit_plans::load_edit_plans_from_disk(&data_root);

        let (shutdown_tx, _) = broadcast::channel(8);
        let (lsp_diag_broadcaster, _) = broadcast::channel(2048);
        let ask_user_question = Arc::new(AskUserQuestionBroker::new());
        let lsp = Arc::new(LspManager::new(lsp_cfg.clone()));
        let telemetry = Telemetry::new(data_root.clone());
        let ops_events = OpsEvents::new(data_root.clone());
        let perf_telemetry = PerfTelemetry::new(data_root.clone());
        let runtime_events = Arc::new(CtxRuntimeEventSink::new(ops_events.clone()));
        let harness_runtime = Arc::new(HarnessRuntimeManager::new_with_event_sink(
            data_root.clone(),
            runtime_events.clone(),
        ));
        let runtime_metrics = Arc::new(CtxRuntimeMetricsSink::new(perf_telemetry.clone()));
        let execution_harness = Arc::new(CtxExecutionHarness::new(harness_runtime.clone()));
        let warmup_operations = Arc::new(DefaultWarmupOperations::new(
            data_root.clone(),
            runtime_events.clone(),
        ));
        let storage_guard = crate::storage_guard::StorageGuardRuntime::new(&data_root);
        let running_sessions = Arc::new(Mutex::new(HashSet::new()));
        let terminals = Arc::new(TerminalManager::default());
        let execution_setup = Arc::new(ExecutionSetupCoordinator::new_with_operations(
            data_root.clone(),
            execution_harness,
            runtime_events,
            runtime_metrics,
            warmup_operations,
        ));
        #[cfg(not(test))]
        {
            let execution_setup = Arc::clone(&execution_setup);
            let startup_data_root = data_root.clone();
            tokio::spawn(async move {
                let db_path = startup_data_root.join("db").join("db.sqlite");
                match Store::open_sqlite(&db_path, None).await {
                    Ok(store) => {
                        let loaded = crate::settings::load_settings(&store).await;
                        store.close().await;
                        match loaded {
                            Ok(settings) => {
                                execution_setup
                                    .spawn_startup_prewarm(settings.execution.unwrap_or_default());
                            }
                            Err(err) => {
                                execution_setup
                                    .record_startup_prewarm_error(format!(
                                        "failed to load execution settings: {err:#}"
                                    ))
                                    .await;
                            }
                        }
                    }
                    Err(err) => {
                        execution_setup
                            .record_startup_prewarm_error(format!(
                                "failed to open global settings store: {err:#}"
                            ))
                            .await;
                    }
                }
            });
        }
        let workspace_active_snapshot = Arc::new(WorkspaceActiveSnapshotHub::new());
        let web_sessions = Arc::new(WebSessionManager::new());
        Self {
            core: CoreState {
                data_root,
                storage_guard,
                tool_output_spool_enabled,
                tool_output_spool_dir,
                stores,
                daemon_url,
                auth_token,
                lsp_cfg,
                lsp,
                lsp_edit_plans_enabled,
                buffers: BufferStore::default(),
                ask_user_question,
                shutdown_tx,
                update_drain: Arc::new(Mutex::new(None)),
            },
            sessions: SessionRuntime {
                session_head_cache: Mutex::new(HashMap::new()),
                schedulers: Mutex::new(HashMap::new()),
                provider_inactivity_timeout: Mutex::new(provider_inactivity_timeout_from_env()),
                broadcasters: Mutex::new(HashMap::new()),
                session_event_heads: Mutex::new(HashMap::new()),
                order_seq_states: Mutex::new(HashMap::new()),
                active_task_refreshes: Mutex::new(HashMap::new()),
                task_session_creation_locks: Mutex::new(HashMap::new()),
                running_sessions,
                session_pins: Mutex::new(HashMap::new()),
                session_meta_cache: Mutex::new(HashMap::new()),
            },
            workspaces: WorkspaceRuntime {
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
                worktree_vcs_scheduler: WorktreeVcsSchedulerRuntime {
                    started: AtomicBool::new(false),
                    notify: Arc::new(Notify::new()),
                    permits: Arc::new(
                        Semaphore::new(worktree_vcs_scheduler_concurrency_from_env()),
                    ),
                },
                git_status_watchers: Mutex::new(HashSet::new()),
                workspace_active_snapshot,
                workspace_active_snapshot_cache: Mutex::new(HashMap::new()),
                workspace_active_heads_cache: Mutex::new(HashMap::new()),
                worktree_bootstrap_gates: Mutex::new(HashMap::new()),
                attachment_materializations: Mutex::new(HashMap::new()),
                attachment_materialization_generation: AtomicU64::new(0),
                edit_plans: Mutex::new(edit_plans),
            },
            providers: ProviderRuntime {
                adapters: Mutex::new(providers),
                target_adapters: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                matrix_cache: Mutex::new(crate::provider_matrix::ProviderMatrixCache::default()),
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
            },
            telemetry: TelemetryRuntime {
                telemetry,
                ops_events,
                perf_telemetry,
                resource_governance: Mutex::new(ResourceGovernanceRuntime::default()),
                resource_sampler: Mutex::new(ResourceSampler::new()),
            },
            transport: TransportRuntime {
                terminals,
                mobile_tunnel: MobileTunnelManager::default(),
                web_sessions,
                merge_queue: Arc::new(ctx_merge_queue::MergeQueueRuntime::new()),
                lsp_diag_broadcaster,
                lsp_diag_forwarders: Mutex::new(HashSet::new()),
            },
            execution: ExecutionRuntime {
                harness: harness_runtime,
                setup: execution_setup,
            },
        }
    }
}
