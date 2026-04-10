use super::*;

#[derive(Debug, Clone, Default)]
pub struct ExecutionSettingsOverride {
    pub mode: Option<ExecutionMode>,
    pub container: ContainerExecutionSettingsOverride,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerExecutionSettingsOverride {
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn load_execution_settings_override(
    store: &Store,
) -> Result<Option<ExecutionSettingsOverride>> {
    let cfg = load_workspace_settings_doc(store).await?;
    let Some(exec) = cfg.execution else {
        return Ok(None);
    };

    let mut ov = ExecutionSettingsOverride::default();
    let environment = exec.environment;
    if let Some(environment) = environment {
        match environment {
            ExecutionEnvironment::Host => {
                ov.mode = Some(ExecutionMode::Host);
            }
            ExecutionEnvironment::Sandbox => {
                ov.mode = Some(ExecutionMode::Sandbox);
            }
        }
    }

    if let Some(c) = exec.container {
        ov.container.network_mode = c.network_mode;
        ov.container.allowlist = c.allowlist.map(|v| {
            v.into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });
        ov.container.image = c
            .image
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
    }

    Ok(Some(ov))
}

pub fn apply_execution_settings_override(
    settings: &mut ExecutionSettings,
    ov: &ExecutionSettingsOverride,
) {
    if let Some(mode) = ov.mode.clone() {
        settings.mode = mode;
    }
    if let Some(network_mode) = ov.container.network_mode.clone() {
        settings.container.network_mode = network_mode;
    }
    if let Some(allowlist) = ov.container.allowlist.clone() {
        settings.container.allowlist = allowlist;
    }
    if let Some(image) = ov.container.image.clone() {
        settings.container.image = Some(image);
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionConfigUpdate {
    pub environment: ExecutionEnvironment,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Option<Vec<String>>,
    pub image: Option<String>,
}

pub async fn update_execution_config(store: &Store, update: ExecutionConfigUpdate) -> Result<()> {
    let container = if matches!(update.environment, ExecutionEnvironment::Sandbox) {
        Some(WorkspaceContainerExecutionConfig {
            network_mode: update.network_mode,
            allowlist: update.allowlist.map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            }),
            image: update
                .image
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        })
    } else {
        None
    };
    mutate_workspace_settings_doc(store, "execution", move |cfg| {
        cfg.execution = Some(WorkspaceExecutionConfig {
            environment: Some(update.environment),
            container,
        });
        Ok(())
    })
    .await
}
