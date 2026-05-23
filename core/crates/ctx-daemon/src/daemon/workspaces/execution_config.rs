use ctx_core::ids::WorkspaceId;
use ctx_route_contracts::workspaces::{
    UpdateWorkspaceExecutionConfigRequest, WorkspaceExecutionConfigRouteSnapshot,
    WorkspaceRouteParams,
};
use ctx_workspace_config as workspace_config;

use super::route_config::{
    request_or_policy_route_error, workspace_execution_config_route_snapshot,
    workspace_store_route_error, WorkspaceConfigUpdateResult, WorkspaceRouteError,
};
use crate::daemon::WorkspaceExecutionConfigHandle;

impl WorkspaceExecutionConfigHandle {
    pub async fn workspace_execution_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceExecutionConfigRouteSnapshot, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_execution_config_for_request(workspace_id)
            .await
    }

    pub async fn update_workspace_execution_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: UpdateWorkspaceExecutionConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.update_workspace_execution_config_for_request(workspace_id, request)
            .await
    }

    pub async fn workspace_execution_config_update_target_for_route_params(
        &self,
        params: &WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        Ok(())
    }

    async fn workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceExecutionConfigRouteSnapshot, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        let settings = ctx_settings_service::load_settings(self.global_store())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        match ctx_settings_service::workspace_execution_config_snapshot_for_loaded_settings(
            &settings, &store,
        )
        .await
        {
            Ok(snapshot) => Ok(workspace_execution_config_route_snapshot(snapshot)),
            Err(
                ctx_settings_service::WorkspaceExecutionConfigSnapshotError::InvalidWorkspaceConfig(
                    error,
                ),
            ) => Err(WorkspaceRouteError::bad_request(error)),
            Err(ctx_settings_service::WorkspaceExecutionConfigSnapshotError::RequestOrPolicy(
                error,
            )) => Err(request_or_policy_route_error(error)),
            Err(ctx_settings_service::WorkspaceExecutionConfigSnapshotError::Internal(error)) => {
                Err(WorkspaceRouteError::internal(error))
            }
        }
    }

    async fn update_workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspaceExecutionConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        let update = workspace_config::parse_execution_config_update_input(
            req.environment.trim(),
            req.network_mode.as_deref(),
            req.allowlist,
            self.sandbox_runtime_available_for_execution_config(),
        )
        .map_err(WorkspaceRouteError::bad_request)?;
        let settings = ctx_settings_service::load_settings(self.global_store())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        ctx_settings_service::update_workspace_execution_config_for_loaded_settings(
            &settings, &store, update,
        )
        .await
        .map_err(|error| match error {
            ctx_settings_service::WorkspaceExecutionConfigUpdateError::RequestOrPolicy(error) => {
                request_or_policy_route_error(error)
            }
            ctx_settings_service::WorkspaceExecutionConfigUpdateError::Persistence(error) => {
                WorkspaceRouteError::bad_request(error)
            }
        })?;
        Ok(WorkspaceConfigUpdateResult { ok: true })
    }

    fn sandbox_runtime_available_for_execution_config(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            ctx_harness_runtime::local_runtime_available(
                self.data_root(),
                &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = self.data_root();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use ctx_core::ids::WorkspaceId;
    use ctx_core::models::{VcsKind, Workspace};
    use ctx_route_contracts::workspaces::{
        UpdateWorkspaceExecutionConfigRequest, WorkspaceRouteErrorKind, WorkspaceRouteParams,
    };

    use crate::test_support::TestDaemon;

    async fn test_daemon() -> (tempfile::TempDir, TestDaemon) {
        let temp = tempfile::tempdir().expect("tempdir");
        let daemon =
            TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
                .await
                .expect("test daemon");
        (temp, daemon)
    }

    async fn create_workspace(daemon: &TestDaemon, name: &str) -> Workspace {
        daemon
            .global_store()
            .create_workspace(
                name.to_string(),
                daemon.data_root().join(name).to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .expect("create workspace")
    }

    #[tokio::test]
    async fn execution_config_route_rejects_invalid_workspace_id() {
        let (_temp, daemon) = test_daemon().await;
        let handle = daemon.handle().workspace_execution_config();

        let get_error = handle
            .workspace_execution_config_for_route_params(WorkspaceRouteParams::new(
                "not-a-workspace",
            ))
            .await
            .unwrap_err();
        assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(get_error.message(), "invalid workspace id");

        let post_error = handle
            .update_workspace_execution_config_for_route_params(
                WorkspaceRouteParams::new("not-a-workspace"),
                UpdateWorkspaceExecutionConfigRequest {
                    environment: "host".to_string(),
                    network_mode: None,
                    allowlist: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(post_error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(post_error.message(), "invalid workspace id");
    }

    #[tokio::test]
    async fn execution_config_route_maps_missing_workspace_to_not_found() {
        let (_temp, daemon) = test_daemon().await;
        let missing_workspace_id = WorkspaceId::new();
        let handle = daemon.handle().workspace_execution_config();

        let get_error = handle
            .workspace_execution_config_for_route_params(WorkspaceRouteParams::new(
                missing_workspace_id.0.to_string(),
            ))
            .await
            .unwrap_err();
        assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(get_error.message(), "workspace not found");

        let post_error = handle
            .update_workspace_execution_config_for_route_params(
                WorkspaceRouteParams::new(missing_workspace_id.0.to_string()),
                UpdateWorkspaceExecutionConfigRequest {
                    environment: "container".to_string(),
                    network_mode: Some("invalid-network".to_string()),
                    allowlist: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(post_error.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(post_error.message(), "workspace not found");
    }

    #[tokio::test]
    async fn execution_config_route_treats_deleting_workspace_as_not_found() {
        let (_temp, daemon) = test_daemon().await;
        let workspace = create_workspace(&daemon, "deleting-execution-config").await;
        daemon.stores().begin_workspace_delete(workspace.id).await;
        let handle = daemon.handle().workspace_execution_config();

        let get_error = handle
            .workspace_execution_config_for_route_params(WorkspaceRouteParams::new(
                workspace.id.0.to_string(),
            ))
            .await
            .unwrap_err();
        assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(get_error.message(), "workspace not found");

        let post_error = handle
            .update_workspace_execution_config_for_route_params(
                WorkspaceRouteParams::new(workspace.id.0.to_string()),
                UpdateWorkspaceExecutionConfigRequest {
                    environment: "host".to_string(),
                    network_mode: None,
                    allowlist: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(post_error.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(post_error.message(), "workspace not found");
        daemon.stores().finish_workspace_delete(workspace.id).await;
    }

    #[tokio::test]
    async fn execution_config_route_maps_unavailable_workspace_store_to_internal() {
        let (_temp, daemon) = test_daemon().await;
        let workspace = create_workspace(&daemon, "unavailable-execution-config").await;
        daemon
            .cache_rehydration_make_workspace_store_unopenable_for_test(workspace.id)
            .await
            .expect("block workspace store");
        let handle = daemon.handle().workspace_execution_config();

        let get_error = handle
            .workspace_execution_config_for_route_params(WorkspaceRouteParams::new(
                workspace.id.0.to_string(),
            ))
            .await
            .unwrap_err();
        assert_eq!(get_error.kind(), WorkspaceRouteErrorKind::Internal);

        let post_error = handle
            .update_workspace_execution_config_for_route_params(
                WorkspaceRouteParams::new(workspace.id.0.to_string()),
                UpdateWorkspaceExecutionConfigRequest {
                    environment: "host".to_string(),
                    network_mode: None,
                    allowlist: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(post_error.kind(), WorkspaceRouteErrorKind::Internal);
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn execution_config_sandbox_runtime_is_available_on_non_macos() {
        let (_temp, daemon) = test_daemon().await;
        assert!(daemon
            .handle()
            .workspace_execution_config()
            .sandbox_runtime_available_for_execution_config());
    }
}
