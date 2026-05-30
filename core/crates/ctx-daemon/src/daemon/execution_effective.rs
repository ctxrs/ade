use ctx_settings_model::ExecutionSettings;

pub async fn effective_execution_settings_for_environment_parts(
    global_store: &ctx_store::Store,
    workspace_store: &ctx_store::Store,
    execution_environment: ctx_core::models::ExecutionEnvironment,
) -> anyhow::Result<ExecutionSettings> {
    ctx_settings_service::effective_execution_settings_for_environment(
        global_store,
        workspace_store,
        execution_environment,
    )
    .await
}

#[cfg(test)]
mod execution_effective_test;
