use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::Mutex;

pub mod model_preferences;
pub mod provider_adapters;
pub mod provider_auth;
pub mod provider_child_reclassifier;
pub mod provider_guard;
pub mod provider_launch;
pub mod provider_restart;
pub mod provider_usability;
pub mod provider_usage;
pub mod resource_governance;

pub trait ProviderRuntimeHost: Send + Sync + 'static {
    fn data_root(&self) -> &Path;

    fn current_ctx_version(&self) -> Option<String>;

    fn provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>;

    fn target_provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>;

    fn provider_statuses(&self) -> &Mutex<HashMap<String, ProviderStatus>>;
}

pub type AppState = dyn ProviderRuntimeHost;
