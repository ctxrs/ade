#[derive(Debug)]
pub enum WorkspaceProviderModelPreferenceError {
    ProviderIdRequired,
    ProviderNotFound { provider_id: String },
    WorkspaceNotFound,
    StoreUnavailable(anyhow::Error),
    ExecutionSettings(anyhow::Error),
}

pub struct WorkspaceProviderModelPreference {
    pub provider_id: String,
    pub preferred_model_id: Option<String>,
}
