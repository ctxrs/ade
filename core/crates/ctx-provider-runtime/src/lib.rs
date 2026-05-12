use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::{InstallId, InstallState};
use ctx_provider_matrix::ProviderMatrixCache;
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::Mutex;

pub mod login_sessions;
pub mod model_preferences;
pub mod provider_adapters;
pub mod provider_auth;
pub mod provider_cache;
pub mod provider_child_reclassifier;
pub mod provider_guard;
pub mod provider_launch;
pub mod provider_restart;
pub mod provider_state;
pub mod provider_usability;
pub mod provider_usage;
pub mod resource_governance;

pub trait ProviderRuntimeHost: Send + Sync + 'static {
    fn data_root(&self) -> &Path;

    fn current_ctx_version(&self) -> Option<String>;

    fn provider_runtime(&self) -> &ProviderRuntime;
}

pub type AppState = dyn ProviderRuntimeHost;

pub struct CachedProviderOptions {
    pub cached_at: Instant,
    pub value: serde_json::Value,
}

pub struct CachedProviderVerify {
    pub cached_at: Instant,
    pub value: serde_json::Value,
}

pub struct ProviderRuntime {
    adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    target_adapters: Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>,
    statuses: Mutex<HashMap<String, ProviderStatus>>,
    pub matrix_cache: Mutex<ProviderMatrixCache>,
    options_cache: Mutex<HashMap<String, CachedProviderOptions>>,
    verify_cache: Mutex<HashMap<String, CachedProviderVerify>>,
    pub guard: Mutex<provider_guard::ProviderGuardRuntime>,
    pub restart: Mutex<provider_restart::ProviderRestartRuntime>,
    usage_cache: Mutex<HashMap<String, provider_usage::ProviderUsageSnapshot>>,
    codex_login_sessions: Mutex<HashMap<String, provider_accounts::CodexLoginStatus>>,
    claude_login_sessions: Mutex<HashMap<String, provider_accounts::ClaudeLoginStatus>>,
    gemini_login_sessions: Mutex<HashMap<String, provider_accounts::GeminiLoginStatus>>,
    qwen_login_sessions: Mutex<HashMap<String, provider_accounts::QwenLoginStatus>>,
    kimi_login_sessions: Mutex<HashMap<String, provider_accounts::KimiLoginStatus>>,
    cursor_login_sessions: Mutex<HashMap<String, provider_accounts::CursorLoginStatus>>,
    amp_login_sessions: Mutex<HashMap<String, provider_accounts::AmpLoginStatus>>,
    mistral_login_sessions: Mutex<HashMap<String, provider_accounts::MistralLoginStatus>>,
    pub install_start_gate: Mutex<()>,
    pub installs: Mutex<HashMap<InstallId, InstallState>>,
}

impl ProviderRuntime {
    pub fn new(providers: HashMap<String, Arc<dyn ProviderAdapter>>) -> Self {
        Self {
            adapters: Mutex::new(providers),
            target_adapters: Mutex::new(HashMap::new()),
            statuses: Mutex::new(HashMap::new()),
            matrix_cache: Mutex::new(ProviderMatrixCache::default()),
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
        }
    }
}
