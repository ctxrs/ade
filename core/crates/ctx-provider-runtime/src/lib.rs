use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::Mutex;

pub mod provider_launch;
pub mod provider_usability;

pub trait ProviderRuntimeHost: Send + Sync + 'static {
    fn data_root(&self) -> &Path;

    fn provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>;

    fn target_provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>;

    fn provider_statuses(&self) -> &Mutex<HashMap<String, ProviderStatus>>;
}

pub type AppState = dyn ProviderRuntimeHost;
