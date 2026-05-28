use super::*;

impl RouteBuilder {
    pub(super) fn task_worktree_host(&self) -> Arc<TaskWorktreeHost> {
        let workspace_stores = self.protected_workspace_store_lookup();
        let attachments = self.workspace_attachments_runtime();
        let vcs_hooks = Arc::new(
            crate::daemon::workspaces::vcs_hooks::WorkspaceVcsHookHost::new(
                self.state.core.data_root.clone(),
                self.state.core.daemon_url.clone(),
                self.state.global_store().clone(),
                workspace_stores.clone(),
                Arc::clone(&self.state.execution.harness),
            ),
        );
        Arc::new(TaskWorktreeHost::new(TaskWorktreeHostParts {
            data_root: self.state.core.data_root.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            global_store: self.state.global_store().clone(),
            workspace_stores,
            harness: Arc::clone(&self.state.execution.harness),
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            bootstrap_gates: Arc::clone(&self.state.workspaces.worktree_bootstrap_gates),
            attachments,
            vcs_hooks,
        }))
    }
    fn task_lifecycle_effects(&self) -> Arc<TaskLifecycleEffects> {
        crate::daemon::task_session_effects::task_lifecycle_effects(
            self.task_publication_host(),
            self.task_session_cleanup_host(),
        )
    }
    pub fn task_lifecycle(&self) -> TaskLifecycleHandle {
        TaskLifecycleHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            self.task_worktree_host(),
            self.task_lifecycle_effects(),
        )
    }
    fn task_store_lookup(&self) -> TaskStoreLookup {
        TaskStoreLookup::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }
    fn task_metadata_effects(&self) -> Arc<TaskMetadataEffects> {
        crate::daemon::task_session_effects::task_metadata_effects(self.task_publication_host())
    }
    pub fn task_listing(&self) -> TaskListingHandle {
        let snapshot = Arc::clone(&self.state.workspaces.workspace_active_snapshot);
        let archived_rev_loader: TaskArchivedRevLoader = Arc::new(move |workspace_id| {
            let snapshot = Arc::clone(&snapshot);
            Box::pin(async move {
                let (_, archived_rev) = snapshot.snapshot_state(workspace_id).await;
                archived_rev
            })
        });
        TaskListingHandle::new(self.protected_workspace_store_lookup(), archived_rev_loader)
    }
    pub fn task_session_listing(&self) -> TaskSessionListingHandle {
        TaskSessionListingHandle::new(self.task_store_lookup())
    }
    pub fn task_read_state(&self) -> TaskReadStateHandle {
        TaskReadStateHandle::new(self.task_store_lookup(), self.task_metadata_effects())
    }
    pub fn task_title(&self) -> TaskTitleHandle {
        let web_sessions = Arc::clone(&self.state.transport.web_sessions);
        let close_web_sessions_for_task: TaskCloseWebSessionsForTask =
            Arc::new(move |session_ids, worktree_ids| {
                let web_sessions = Arc::clone(&web_sessions);
                Box::pin(async move {
                    web_sessions
                        .close_for_task(&session_ids, &worktree_ids)
                        .await
                })
            });
        TaskTitleHandle::new(
            self.task_store_lookup(),
            self.task_metadata_effects(),
            close_web_sessions_for_task,
        )
    }
    fn task_session_admission_effects_with_title_mode(
        &self,
        session_title_model_mode: SessionTitleModelModeHandle,
    ) -> Arc<TaskAdmissionSessionEffects> {
        crate::daemon::task_session_effects::task_admission_session_effects(
            self.session_publication_effects(),
            Arc::clone(&self.state.sessions),
            SessionMessageSchedulerSpawner::new(Arc::downgrade(
                &self.state.session_scheduler_worker_host.worker_host(),
            )),
            session_title_model_mode,
            self.task_publication_host(),
        )
    }
    fn task_session_admission_provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    fn task_session_admission_model_catalog_loader_with_provider_routes(
        &self,
        provider_routes: &provider_deps::ProviderRouteDeps,
    ) -> TaskAdmissionModelCatalogLoader {
        let launch = provider_routes.provider_workspace_launch_runtime();
        Arc::new(
            move |workspace: Workspace,
                  provider_id: String,
                  execution_environment: ExecutionEnvironment| {
                let launch = Arc::clone(&launch);
                Box::pin(async move {
                    crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
                        launch.as_ref(),
                        &workspace,
                        &provider_id,
                        execution_environment,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            },
        )
    }
    pub(super) fn task_session_admission_with_provider_routes(
        &self,
        provider_routes: &provider_deps::ProviderRouteDeps,
        session_title_model_mode: SessionTitleModelModeHandle,
    ) -> TaskSessionAdmissionHandle {
        TaskSessionAdmissionHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.providers),
            self.task_session_admission_provider_status(),
            self.task_worktree_host(),
            self.task_session_admission_effects_with_title_mode(session_title_model_mode),
            self.task_session_admission_model_catalog_loader_with_provider_routes(provider_routes),
            self.state.telemetry.telemetry.clone(),
            self.state.telemetry.ops_events.clone(),
            self.state.telemetry.perf_telemetry.clone(),
        )
    }
    pub(super) fn task_creation_with_session_admission(
        &self,
        task_session_admission: TaskSessionAdmissionHandle,
    ) -> TaskCreationHandle {
        TaskCreationHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            task_session_admission,
            self.task_lifecycle(),
        )
    }
}
