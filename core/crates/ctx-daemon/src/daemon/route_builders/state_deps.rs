use super::route_graph_parts::RouteGraphParts;
use super::*;
use crate::daemon::state::DaemonState;

#[derive(Clone)]
pub(crate) struct RouteBuilder {
    parts: RouteGraphParts,
}

pub(crate) fn route_handles_from_state(state: &Arc<DaemonState>) -> DaemonRouteHandles {
    let handle = RouteBuilder::new(RouteGraphParts::from_state(state));
    route_handles_from_builder(handle)
}

fn route_handles_from_builder(handle: RouteBuilder) -> DaemonRouteHandles {
    let core_routes = handle.core_route_deps();
    let merge_queue_host = handle.merge_queue_route_host();
    let provider_routes = handle.provider_route_deps();
    let workspace_routes = handle.workspace_route_deps(Arc::clone(&merge_queue_host));
    let session_routes = handle.session_route_deps(&workspace_routes);
    let task_routes = handle.task_route_deps();
    let transport_routes = handle.transport_route_deps(core_routes.health());
    let execution_routes = handle.execution_route_deps();
    let maintenance_routes = handle.maintenance_route_deps(merge_queue_host);
    let session_title_model_mode = session_routes.session_title_model_mode();
    let task_session_admission = task_routes.task_session_admission_with_route_deps(
        &provider_routes,
        &session_routes,
        session_title_model_mode.clone(),
    );
    DaemonRouteHandles {
        auth: core_routes.auth(),
        health: core_routes.health(),
        diagnostics: core_routes.diagnostics(),
        blob: core_routes.blob(),
        request_base: core_routes.request_base(),
        repo_onboarding: core_routes.repo_onboarding(),
        logs: core_routes.logs(),
        org_policy: maintenance_routes.org_policy(),
        workspace_org_policy: workspace_routes.workspace_org_policy(),
        workspace_prompt_bootstrap_config: workspace_routes.workspace_prompt_bootstrap_config(),
        workspace_execution_config: workspace_routes.workspace_execution_config(),
        workspace_file_completions: workspace_routes.workspace_file_completions(),
        workspace_harness_container: workspace_routes.workspace_harness_container(),
        workspace_provider_model_preferences: workspace_routes
            .workspace_provider_model_preferences_with_provider_routes(&provider_routes),
        workspace_worktree: workspace_routes.workspace_worktree(),
        workspace_registry: workspace_routes.workspace_registry(),
        workspace_merge_queue_config: workspace_routes.workspace_merge_queue_config(),
        merge_queue_api: maintenance_routes.merge_queue_api(),
        workspace_attachments: workspace_routes.workspace_attachments(),
        workspace_primary_branch: workspace_routes.workspace_primary_branch(),
        dictation: core_routes.dictation(),
        update_release: maintenance_routes.update_release(),
        update_activity: maintenance_routes.update_activity(),
        settings: maintenance_routes.settings(),
        mobile_store: transport_routes.mobile_store(),
        mobile_runtime: transport_routes.mobile_runtime(),
        mobile_secure_proxy: transport_routes.mobile_secure_proxy(),
        resource_utilization: maintenance_routes.resource_utilization(),
        run_archive: maintenance_routes.run_archive(),
        session_artifacts: session_routes.session_artifacts(),
        session_control: session_routes.session_control_with_provider_routes(&provider_routes),
        session_file_completions: session_routes.session_file_completions(),
        session_message_command: session_routes.session_message_command(),
        session_read_models: session_routes.session_read_models(),
        session_subagent_mcp_read: session_routes.session_subagent_mcp_read(),
        session_subagent_mcp_control: session_routes
            .session_subagent_mcp_control_with_provider_routes(&provider_routes),
        session_subagent_read: session_routes.session_subagent_read(),
        session_title_model_mode,
        session_vcs: session_routes.session_vcs(),
        demo_seed_transcript: session_routes.demo_seed_transcript(),
        title_generation_local: session_routes.title_generation_local(),
        task_creation: task_routes
            .task_creation_with_session_admission(task_session_admission.clone(), &session_routes),
        task_lifecycle: task_routes.task_lifecycle_with_session_routes(&session_routes),
        task_listing: task_routes.task_listing(),
        task_read_state: task_routes.task_read_state_with_session_routes(&session_routes),
        task_session_admission,
        task_session_listing: task_routes.task_session_listing(),
        task_title: task_routes.task_title_with_session_routes(&session_routes),
        workspace_deletion: workspace_routes.workspace_deletion(),
        workspace_active: workspace_routes.workspace_active(),
        workspace_stream: workspace_routes.workspace_stream(),
        workspace_vcs_stream: workspace_routes.workspace_vcs_stream(),
        provider_accounts: provider_routes.provider_accounts(),
        provider_auth_import: provider_routes.provider_auth_import(),
        provider_status: provider_routes.provider_status(),
        provider_admin: provider_routes.provider_admin(),
        provider_install: provider_routes.provider_install(),
        provider_usage: provider_routes.provider_usage(),
        provider_harness_config: provider_routes.provider_harness_config(),
        provider_bootstrap: provider_routes.provider_bootstrap(),
        provider_options: provider_routes.provider_options(),
        provider_workspace_auth: provider_routes.provider_workspace_auth(),
        telemetry: maintenance_routes.telemetry(),
        terminal_route: transport_routes.terminal_route(),
        web_session_route: transport_routes.web_session_route(),
        execution_launch: execution_routes.execution_launch(),
        linux_sandbox_runtime: execution_routes.linux_sandbox_runtime(),
        update_drain: maintenance_routes.update_drain(),
        daemon_shutdown: maintenance_routes.daemon_shutdown_with_session_routes(&session_routes),
    }
}

impl RouteBuilder {
    pub(crate) fn new(parts: RouteGraphParts) -> Self {
        Self { parts }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn from_state_for_test_support(state: &Arc<DaemonState>) -> Self {
        Self::new(RouteGraphParts::from_state(state))
    }

    pub(super) fn core_route_deps(&self) -> core_deps::CoreRouteDeps {
        core_deps::CoreRouteDeps::new(core_deps::CoreRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            public_base_url: self.parts.public_base_url.clone(),
            auth_token: self.parts.auth_token.clone(),
            mcp_auth: Arc::clone(&self.parts.mcp_auth),
            storage_guard: Arc::clone(&self.parts.storage_guard),
            global_store: self.parts.global_store.clone(),
            ops_events: self.parts.ops_events.clone(),
            execution_setup: Arc::clone(&self.parts.execution_setup),
            providers: Arc::clone(&self.parts.providers),
        })
    }

    pub(super) fn execution_route_deps(&self) -> execution_deps::ExecutionRouteDeps {
        execution_deps::ExecutionRouteDeps::new(execution_deps::ExecutionRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            global_store: self.parts.global_store.clone(),
            stores: self.parts.stores.clone(),
            update_drain: Arc::clone(&self.parts.update_drain),
            execution_setup: Arc::clone(&self.parts.execution_setup),
            harness: Arc::clone(&self.parts.harness),
            terminals: Arc::clone(&self.parts.terminals),
        })
    }

    pub(super) fn maintenance_route_deps(
        &self,
        merge_queue_host: Arc<crate::daemon::merge_queue::MergeQueueRouteHost>,
    ) -> maintenance_deps::MaintenanceRouteDeps {
        maintenance_deps::MaintenanceRouteDeps::new(maintenance_deps::MaintenanceRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            global_store: self.parts.global_store.clone(),
            stores: self.parts.stores.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            merge_queue_host,
            telemetry: self.parts.telemetry.clone(),
            perf_telemetry: self.parts.perf_telemetry.clone(),
            resource_sampler: Arc::clone(&self.parts.resource_sampler),
            resource_governance: Arc::clone(&self.parts.resource_governance),
            providers: Arc::clone(&self.parts.providers),
            terminals: Arc::clone(&self.parts.terminals),
            update_drain: Arc::clone(&self.parts.update_drain),
            sessions: Arc::clone(&self.parts.sessions),
            harness: Arc::clone(&self.parts.harness),
            shutdown_tx: self.parts.shutdown_tx.clone(),
            local_shutdown_token: self.parts.local_shutdown_token.clone(),
        })
    }

    pub(super) fn merge_queue_route_host(
        &self,
    ) -> Arc<crate::daemon::merge_queue::MergeQueueRouteHost> {
        Arc::clone(&self.parts.merge_queue_host)
    }

    pub(super) fn provider_route_deps(&self) -> provider_deps::ProviderRouteDeps {
        provider_deps::ProviderRouteDeps::new(provider_deps::ProviderRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            auth_token: self.parts.auth_token.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            providers: Arc::clone(&self.parts.providers),
            ops_events: self.parts.ops_events.clone(),
            shutdown_tx: self.parts.shutdown_tx.clone(),
            harness: Arc::clone(&self.parts.harness),
        })
    }

    pub(super) fn protected_workspace_store_lookup(&self) -> ProtectedWorkspaceStoreLookup {
        self.parts.workspace_stores.clone()
    }

    pub(super) fn task_route_deps(&self) -> task_deps::TaskRouteDeps {
        task_deps::TaskRouteDeps::new(task_deps::TaskRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            global_store: self.parts.global_store.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            active_snapshot: Arc::clone(&self.parts.active_snapshot),
            sessions: Arc::clone(&self.parts.sessions),
            scheduler_worker_host: Arc::clone(&self.parts.scheduler_worker_host),
            providers: Arc::clone(&self.parts.providers),
            web_sessions: Arc::clone(&self.parts.web_sessions),
            telemetry: self.parts.telemetry.clone(),
            ops_events: self.parts.ops_events.clone(),
            perf_telemetry: self.parts.perf_telemetry.clone(),
        })
    }

    pub(super) fn session_route_deps(
        &self,
        workspace_routes: &workspace_deps::WorkspaceRouteDeps,
    ) -> session_deps::SessionRouteDeps {
        session_deps::SessionRouteDeps::new(session_deps::SessionRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            tool_output_spool_dir: self.parts.tool_output_spool_dir.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            auth_token: self.parts.auth_token.clone(),
            global_store: self.parts.global_store.clone(),
            stores: self.parts.stores.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            session_stores: self.parts.session_stores.clone(),
            weak_session_stores: self.parts.weak_session_stores.clone(),
            sessions: Arc::clone(&self.parts.sessions),
            scheduler_worker_host: Arc::clone(&self.parts.scheduler_worker_host),
            active_snapshot: Arc::clone(&self.parts.active_snapshot),
            worktree_file_completions_cache: Arc::clone(
                &self.parts.worktree_file_completions_cache,
            ),
            providers: Arc::clone(&self.parts.providers),
            ops_events: self.parts.ops_events.clone(),
            perf_telemetry: self.parts.perf_telemetry.clone(),
            provider_unknown_events: self.parts.provider_unknown_events.clone(),
            ask_user_question: Arc::clone(&self.parts.ask_user_question),
            update_drain: Arc::clone(&self.parts.update_drain),
            harness: Arc::clone(&self.parts.harness),
            task_publication: Arc::clone(&self.parts.task_publication),
            task_worktree_host: workspace_routes.task_worktree_host(),
            worktree_vcs_runtime: workspace_routes.worktree_vcs_runtime_host(),
            worktree_vcs_execution: workspace_routes.worktree_vcs_execution_host(),
        })
    }

    pub(super) fn transport_route_deps(
        &self,
        health: HealthHandle,
    ) -> transport_deps::TransportRouteDeps {
        transport_deps::TransportRouteDeps::new(transport_deps::TransportRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            auth_token_configured: self.parts.auth_token.is_some(),
            global_store: self.parts.global_store.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            mobile_tunnel: self.parts.mobile_tunnel.clone(),
            terminals: Arc::clone(&self.parts.terminals),
            web_sessions: Arc::clone(&self.parts.web_sessions),
            providers: Arc::clone(&self.parts.providers),
            harness: Arc::clone(&self.parts.harness),
            health,
            telemetry: self.parts.telemetry.clone(),
            ops_events: self.parts.ops_events.clone(),
        })
    }

    pub(super) fn workspace_route_deps(
        &self,
        merge_queue_host: Arc<crate::daemon::merge_queue::MergeQueueRouteHost>,
    ) -> workspace_deps::WorkspaceRouteDeps {
        workspace_deps::WorkspaceRouteDeps::new(workspace_deps::WorkspaceRouteDepsParts {
            data_root: self.parts.data_root.clone(),
            daemon_url: self.parts.daemon_url.clone(),
            stores: self.parts.stores.clone(),
            global_store: self.parts.global_store.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            session_stores: self.parts.session_stores.clone(),
            sessions: Arc::clone(&self.parts.sessions),
            active_snapshot: Arc::clone(&self.parts.active_snapshot),
            workspace_active_snapshot_cache: Arc::clone(
                &self.parts.workspace_active_snapshot_cache,
            ),
            workspace_active_heads_cache: Arc::clone(&self.parts.workspace_active_heads_cache),
            workspace_file_completions_cache: Arc::clone(
                &self.parts.workspace_file_completions_cache,
            ),
            worktree_bootstrap_gates: Arc::clone(&self.parts.worktree_bootstrap_gates),
            attachment_materialization: Arc::clone(&self.parts.attachment_materialization),
            harness: Arc::clone(&self.parts.harness),
            providers: Arc::clone(&self.parts.providers),
            merge_queue: Arc::clone(&self.parts.merge_queue),
            merge_queue_host,
            telemetry: self.parts.telemetry.clone(),
            perf_telemetry: self.parts.perf_telemetry.clone(),
            worktree_vcs_runtime: self.parts.worktree_vcs_runtime.clone(),
            worktree_vcs_execution: self.parts.worktree_vcs_execution.clone(),
            workspace_vcs_stream_runtime: self.parts.workspace_vcs_stream_runtime.clone(),
        })
    }
}
