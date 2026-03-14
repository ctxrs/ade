use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::provider_accounts::CODEX_AUTH_TYPE_BEARER;

const REGISTRY_VERSION: u32 = 1;
const SECRET_VERSION: u32 = 1;

const PROVIDER_CODEX: &str = "codex";
const PROVIDER_CLAUDE: &str = "claude-crp";
const PROVIDER_GEMINI: &str = "gemini";
const PROVIDER_KIMI: &str = "kimi";
const PROVIDER_QWEN: &str = "qwen";
const PROVIDER_OPENCODE: &str = "opencode";
const PROVIDER_MISTRAL: &str = "mistral";
const PROVIDER_GOOSE: &str = "goose";
const PROVIDER_AMP: &str = "amp";
const PROVIDER_DROID: &str = "droid";
const PROVIDER_OPENHANDS: &str = "openhands";
const PROVIDER_COPILOT: &str = "copilot";
const PROVIDER_AUGGIE: &str = "auggie";
const PROVIDER_PI: &str = "pi";
const PROVIDER_CURSOR: &str = "cursor";
const PROVIDER_CLINE: &str = "cline";
const CTX_DROID_HOST_AUTH_PATH_ENV: &str = "CTX_DROID_HOST_AUTH_PATH";

const CLAUDE_AUTH_TYPE_API_KEY: &str = "api_key";
const GEMINI_AUTH_TYPE_GEMINI_API_KEY: &str = "gemini_api_key";
const GEMINI_AUTH_TYPE_VERTEX_AI: &str = "vertex_ai";
const ENDPOINT_MODEL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);
const ENDPOINT_MODEL_CATALOG_TTL: Duration = Duration::from_secs(60 * 60 * 24);
const GENERIC_ENDPOINT_NAMESPACE_LABELS: &[&str] = &[
    "api",
    "www",
    "app",
    "gateway",
    "proxy",
    "chat",
    "inference",
    "llm",
];

static REGISTRY_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

mod model_catalog;
mod registry;
mod runtime_resolution;
mod secrets;
mod selection;
mod validation;

pub use model_catalog::{endpoint_model_catalog_is_stale, endpoint_model_catalog_ttl};
pub use runtime_resolution::{
    resolve_provider_source_for_probe, resolve_provider_source_for_probe_with_runtime_root,
    resolve_provider_source_for_run, resolve_provider_source_for_run_with_runtime_root,
};
pub use selection::{
    delete_provider_endpoint, find_provider_endpoint_import_match, get_provider_source_config,
    mark_endpoint_verification, refresh_provider_endpoint_model_catalog,
    set_provider_endpoint_manual_models, set_provider_source_selection, upsert_provider_endpoint,
};
pub use validation::{
    default_shape_for_provider, ensure_shape_compatible, supports_harness_endpoint,
};

pub(crate) fn droid_cli_model_id_for_endpoint_model(
    model_id: Option<&str>,
    base_url: Option<&str>,
) -> Option<String> {
    runtime_resolution::droid_cli_model_id_for_endpoint_model(model_id, base_url)
}

#[cfg(test)]
use model_catalog::{
    infer_endpoint_model_provider_namespace, normalize_namespaced_model_override,
    parse_openai_models_payload, truncate_discovery_error,
};
#[cfg(test)]
use registry::registry_path;
#[cfg(test)]
use runtime_resolution::{
    cline_endpoint_home, codex_endpoint_home, droid_endpoint_home, gemini_endpoint_home,
    qwen_endpoint_home, seed_droid_auth_from_host_path,
};
#[cfg(test)]
use validation::normalize_manual_model_ids;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSourceKind {
    #[default]
    Subscription,
    Endpoint,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessApiShape {
    OpenaiResponses,
    AnthropicMessages,
}

impl HarnessApiShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessEndpointVerificationStatus {
    Unknown,
    Valid,
    Invalid,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndpointModelCatalogStatus {
    #[default]
    Unknown,
    Ready,
    ManualOnly,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointModelRecord {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessEndpointRecord {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub api_shape: HarnessApiShape,
    pub auth_type: String,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
    pub last_verification_status: HarnessEndpointVerificationStatus,
    #[serde(default)]
    pub last_verification_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub has_api_key: bool,
    #[serde(default)]
    pub model_catalog_status: EndpointModelCatalogStatus,
    #[serde(default)]
    pub model_catalog_fetched_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub model_catalog_error: Option<String>,
    #[serde(default)]
    pub model_catalog_models: Vec<EndpointModelRecord>,
    #[serde(default)]
    pub manual_model_ids: Vec<String>,
    #[serde(default)]
    pub model_catalog_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessProviderSourceConfig {
    pub provider_id: String,
    pub selected_source_kind: HarnessSourceKind,
    #[serde(default)]
    pub selected_endpoint_id: Option<String>,
    #[serde(default)]
    pub endpoints: Vec<HarnessEndpointRecord>,
}

#[derive(Debug, Clone)]
pub struct ResolvedHarnessSource {
    pub source_kind: HarnessSourceKind,
    pub endpoint: Option<HarnessEndpointRecord>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct HarnessEndpointUpsert {
    pub endpoint_id: Option<String>,
    pub name: String,
    pub base_url: Option<String>,
    pub api_shape: Option<HarnessApiShape>,
    pub auth_type: Option<String>,
    pub model_override: Option<String>,
    pub api_key: Option<String>,
    pub service_account_json: Option<String>,
    pub project_id: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessEndpointImportMatchKind {
    ExactCredentials,
    SameConfig,
}

#[derive(Debug, Clone)]
pub struct HarnessEndpointImportMatch {
    pub endpoint_id: String,
    pub kind: HarnessEndpointImportMatchKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessEndpointRecordInternal {
    id: String,
    provider_id: String,
    name: String,
    base_url: String,
    api_shape: HarnessApiShape,
    auth_type: String,
    #[serde(default)]
    model_override: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_verification_status: HarnessEndpointVerificationStatus,
    #[serde(default)]
    last_verification_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    model_catalog_status: EndpointModelCatalogStatus,
    #[serde(default)]
    model_catalog_fetched_at: Option<DateTime<Utc>>,
    #[serde(default)]
    model_catalog_error: Option<String>,
    #[serde(default)]
    model_catalog_models: Vec<EndpointModelRecord>,
    #[serde(default)]
    manual_model_ids: Vec<String>,
    #[serde(default)]
    model_catalog_source: Option<String>,
    secret_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HarnessProviderConfigInternal {
    #[serde(default)]
    selected_source_kind: HarnessSourceKind,
    #[serde(default)]
    selected_endpoint_id: Option<String>,
    #[serde(default)]
    endpoints: Vec<HarnessEndpointRecordInternal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessSourceRegistryInternal {
    version: u32,
    #[serde(default)]
    providers: BTreeMap<String, HarnessProviderConfigInternal>,
}

impl Default for HarnessSourceRegistryInternal {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            providers: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests;
