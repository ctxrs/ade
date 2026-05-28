use super::*;

#[cfg(test)]
use crate::daemon::workspace_route_handles::WorkspacePrimaryBranchRefreshEffect;
#[cfg(test)]
use crate::daemon::workspace_stream_route_handles::WorkspaceVcsStreamRefreshEffect;

impl RouteBuilder {
    pub fn workspace_registry(&self) -> WorkspaceRegistryHandle {
        WorkspaceRegistryHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.telemetry.telemetry.clone(),
        )
    }
    pub fn workspace_deletion(&self) -> WorkspaceDeletionHandle {
        WorkspaceDeletionHandle::new(Arc::new(
            crate::daemon::workspaces::WorkspaceDeletionRuntime::new(
                WorkspaceDeletionRuntimeDeps {
                    data_root: self.state.core.data_root.clone(),
                    daemon_url: self.state.core.daemon_url.clone(),
                    stores: self.state.core.stores.clone(),
                    global_store: self.state.global_store().clone(),
                    sessions: Arc::clone(&self.state.sessions),
                    active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
                    workspace_active_snapshot_cache: Arc::clone(
                        &self.state.workspaces.workspace_active_snapshot_cache,
                    ),
                    workspace_active_heads_cache: Arc::clone(
                        &self.state.workspaces.workspace_active_heads_cache,
                    ),
                    workspace_file_completions_cache: Arc::clone(
                        &self.state.workspaces.workspace_file_completions_cache,
                    ),
                    harness: Arc::clone(&self.state.execution.harness),
                    providers: Arc::clone(&self.state.providers),
                    merge_queue: Arc::clone(&self.state.transport.merge_queue),
                },
            ),
        ))
    }
    pub fn workspace_merge_queue_config(&self) -> WorkspaceMergeQueueConfigHandle {
        WorkspaceMergeQueueConfigHandle::new(
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }
    pub(super) fn worktree_vcs_execution_host(&self) -> WorktreeVcsExecutionHost {
        WorktreeVcsExecutionHost::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.execution.harness),
        )
    }
    pub(super) fn worktree_vcs_runtime_host(&self) -> WorktreeVcsRuntimeHost {
        WorktreeVcsRuntimeHost::from_workspace_runtime(&self.state.workspaces)
    }
    pub fn workspace_attachments(&self) -> WorkspaceAttachmentsHandle {
        let workspace_stores = self.protected_workspace_store_lookup();
        WorkspaceAttachmentsHandle::new(
            self.state.global_store().clone(),
            workspace_stores,
            self.workspace_attachments_runtime(),
        )
    }
    pub fn workspace_primary_branch(&self) -> WorkspacePrimaryBranchHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let refresh_vcs_snapshot = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
                        &vcs_runtime,
                        &vcs_execution,
                        &worktree,
                        true,
                    )
                    .await
                }) as WorkspacePrimaryBranchRefreshFuture
            }
        });
        WorkspacePrimaryBranchHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            refresh_vcs_snapshot,
        )
    }
    #[cfg(test)]
    pub(in crate::daemon) fn workspace_primary_branch_with_refresh_effect(
        &self,
        refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
    ) -> WorkspacePrimaryBranchHandle {
        WorkspacePrimaryBranchHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            refresh_vcs_snapshot,
        )
    }
    pub fn workspace_org_policy(&self) -> WorkspaceOrgPolicyHandle {
        WorkspaceOrgPolicyHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }
    pub fn workspace_prompt_bootstrap_config(&self) -> WorkspacePromptBootstrapConfigHandle {
        WorkspacePromptBootstrapConfigHandle::new(self.protected_workspace_store_lookup())
    }
    pub fn workspace_file_completions(&self) -> WorkspaceFileCompletionsHandle {
        WorkspaceFileCompletionsHandle::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.workspaces.workspace_file_completions_cache),
            self.state.telemetry.perf_telemetry.clone(),
        )
    }
    pub fn workspace_execution_config(&self) -> WorkspaceExecutionConfigHandle {
        WorkspaceExecutionConfigHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
        )
    }
    pub fn workspace_harness_container(&self) -> WorkspaceHarnessContainerHandle {
        WorkspaceHarnessContainerHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.execution.harness),
        )
    }
    pub fn workspace_worktree(&self) -> WorkspaceWorktreeHandle {
        WorkspaceWorktreeHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
        )
    }
    pub(super) fn workspace_provider_model_preferences_with_provider_routes(
        &self,
        provider_routes: &provider_deps::ProviderRouteDeps,
    ) -> WorkspaceProviderModelPreferenceHandle {
        WorkspaceProviderModelPreferenceHandle::new(
            provider_routes.provider_workspace_launch_runtime(),
        )
    }
    pub(crate) fn workspace_attachments_runtime(
        &self,
    ) -> Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime> {
        Arc::new(
            crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime::new(
                self.state.core.data_root.clone(),
                self.state.core.daemon_url.clone(),
                self.state.global_store().clone(),
                self.protected_workspace_store_lookup(),
                Arc::clone(&self.state.execution.harness),
                Arc::clone(&self.state.workspaces.attachment_materialization),
            ),
        )
    }
    pub fn workspace_active(&self) -> WorkspaceActiveHandle {
        let hydration = WorkspaceActiveHydrationRuntime::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let cache = WorkspaceActiveCacheRuntime::new(
            Arc::clone(&self.state.workspaces.workspace_active_snapshot_cache),
            Arc::clone(&self.state.workspaces.workspace_active_heads_cache),
        );
        let merge_queue = self.merge_queue_route_host();
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let hydration = hydration.clone();
            move |workspace_id: WorkspaceId| {
                let hydration = hydration.clone();
                Box::pin(async move {
                    hydration
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceActiveFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let merge_queue = Arc::clone(&merge_queue);
            move |workspace_id: WorkspaceId| {
                let merge_queue = Arc::clone(&merge_queue);
                Box::pin(async move {
                    ctx_merge_queue::activate_workspace_merge_queue(&merge_queue, workspace_id)
                        .await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_snapshot = Arc::new({
            let cache = cache.clone();
            move |snapshot: WorkspaceActiveSnapshot| {
                let cache = cache.clone();
                Box::pin(async move {
                    cache.cache_workspace_active_snapshot(snapshot).await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_heads = Arc::new({
            let cache = cache.clone();
            move |heads: WorkspaceActiveHeadBatch| {
                let cache = cache.clone();
                Box::pin(async move {
                    cache.cache_workspace_active_heads(heads).await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        WorkspaceActiveHandle::new(WorkspaceActiveHandleParts {
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            effects: WorkspaceActiveEffects::new(WorkspaceActiveEffectsParts {
                ensure_workspace_active_snapshot_hydrated,
                activate_workspace_merge_queue,
                cache_workspace_active_snapshot,
                cache_workspace_active_heads,
            }),
        })
    }
    pub fn workspace_stream(&self) -> WorkspaceStreamHandle {
        let workspace_stores = ProtectedWorkspaceStoreLookup::new(
            self.state.core.stores.clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        );
        let session_stores =
            SessionStoreLookup::new(self.state.global_store().clone(), workspace_stores.clone());
        let hydration = WorkspaceActiveHydrationRuntime::new(
            self.state.global_store().clone(),
            workspace_stores.clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let merge_queue = self.merge_queue_route_host();
        let lifecycle_host = Arc::new(WorkspaceStreamSessionLifecycleHost::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            Arc::clone(&self.state.providers),
        ));
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let hydration = hydration.clone();
            move |workspace_id: WorkspaceId| {
                let hydration = hydration.clone();
                Box::pin(async move {
                    hydration
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceStreamFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let merge_queue = Arc::clone(&merge_queue);
            move |workspace_id: WorkspaceId| {
                let merge_queue = Arc::clone(&merge_queue);
                Box::pin(async move {
                    ctx_merge_queue::activate_workspace_merge_queue(&merge_queue, workspace_id)
                        .await;
                }) as WorkspaceStreamFuture<_>
            }
        });
        WorkspaceStreamHandle::new(WorkspaceStreamHandleParts {
            global_store: self.state.global_store().clone(),
            workspace_stores,
            session_stores,
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            sessions: Arc::clone(&self.state.sessions),
            lifecycle_host,
            telemetry: self.state.telemetry.telemetry.clone(),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            effects: WorkspaceStreamEffects::new(WorkspaceStreamEffectsParts {
                ensure_workspace_active_snapshot_hydrated,
                activate_workspace_merge_queue,
            }),
        })
    }
    pub fn workspace_vcs_stream(&self) -> WorkspaceVcsStreamHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let ensure_worktree_vcs_watcher = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution, worktree)
                        .await;
                }) as WorkspaceVcsStreamWatcherFuture
            }
        });
        let refresh_worktree_vcs = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, summary: bool, touched_files: bool| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution.clone(), worktree.clone())
                        .await;
                    crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
                        &vcs_runtime,
                        &vcs_execution,
                        &worktree,
                        summary,
                        touched_files,
                    )
                    .await
                }) as WorkspaceVcsStreamRefreshFuture
            }
        });
        WorkspaceVcsStreamHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime::from_workspace_runtime(
                &self.state.workspaces,
            ),
            self.state.telemetry.perf_telemetry.clone(),
            ensure_worktree_vcs_watcher,
            refresh_worktree_vcs,
        )
    }
    #[cfg(test)]
    pub(in crate::daemon) fn workspace_vcs_stream_with_refresh_effect(
        &self,
        refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
    ) -> WorkspaceVcsStreamHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let ensure_worktree_vcs_watcher = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution, worktree)
                        .await;
                }) as WorkspaceVcsStreamWatcherFuture
            }
        });
        WorkspaceVcsStreamHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime::from_workspace_runtime(
                &self.state.workspaces,
            ),
            self.state.telemetry.perf_telemetry.clone(),
            ensure_worktree_vcs_watcher,
            refresh_worktree_vcs,
        )
    }
}
