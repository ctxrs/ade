use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

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
const PROVIDER_CAGENT: &str = "cagent";
const PROVIDER_AMP: &str = "amp";
const PROVIDER_DROID: &str = "droid";
const PROVIDER_CODY: &str = "cody";
const PROVIDER_CONTINUE: &str = "continue";
const PROVIDER_CLINE: &str = "cline";
const PROVIDER_OPENHANDS: &str = "openhands";
const PROVIDER_COPILOT: &str = "copilot";
const PROVIDER_KIRO: &str = "kiro";
const PROVIDER_AUGGIE: &str = "auggie";
const PROVIDER_PI: &str = "pi";
const PROVIDER_CURSOR: &str = "cursor";

const CODEX_AUTH_TYPE_BEARER: &str = "bearer";
const CLAUDE_AUTH_TYPE_API_KEY: &str = "api_key";
const GEMINI_AUTH_TYPE_GEMINI_API_KEY: &str = "gemini_api_key";
const GEMINI_AUTH_TYPE_VERTEX_AI: &str = "vertex_ai";
const KIRO_AUTH_TOKEN_RELATIVE_PATH: &str = ".aws/sso/cache/kiro-auth-token.json";
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointSecretEnvelope {
    version: u32,
    api_key: String,
}

fn registry_path(data_root: &Path) -> PathBuf {
    data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json")
}

fn endpoint_secret_dir(data_root: &Path) -> PathBuf {
    data_root.join("secrets").join("harness_endpoints")
}

fn endpoint_secret_path(data_root: &Path, secret_ref: &str) -> PathBuf {
    endpoint_secret_dir(data_root).join(secret_ref)
}

fn codex_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("codex")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn qwen_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("qwen")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn cline_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("cline")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn kiro_endpoint_home(data_root: &Path, endpoint_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("kiro")
        .join("endpoint-homes")
        .join(endpoint_id)
}

fn amp_subscription_home(data_root: &Path, runtime_data_root: Option<&Path>) -> PathBuf {
    runtime_data_root
        .unwrap_or(data_root)
        .join("providers")
        .join("amp")
        .join("home")
}

fn subscription_env_for_provider(
    canonical: &str,
    data_root: &Path,
    runtime_data_root: Option<&Path>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if canonical == PROVIDER_AMP {
        let home = amp_subscription_home(data_root, runtime_data_root);
        env.insert("HOME".to_string(), home.to_string_lossy().to_string());
        env.insert(
            "XDG_CONFIG_HOME".to_string(),
            home.join(".config").to_string_lossy().to_string(),
        );
        env.insert(
            "XDG_CACHE_HOME".to_string(),
            home.join(".cache").to_string_lossy().to_string(),
        );
    }
    env
}

fn container_workspaces_root(data_root: &Path) -> PathBuf {
    data_root.join("containers").join("workspaces")
}

async fn container_runtime_data_roots(data_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut entries = match tokio::fs::read_dir(container_workspaces_root(data_root)).await {
        Ok(entries) => entries,
        Err(_) => return roots,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let runtime_root = entry.path().join("data");
        match tokio::fs::metadata(&runtime_root).await {
            Ok(metadata) if metadata.is_dir() => roots.push(runtime_root),
            _ => {}
        }
    }

    roots
}

async fn remove_kiro_endpoint_home_for_root(root: &Path, endpoint_id: &str) -> Result<()> {
    let endpoint_home = kiro_endpoint_home(root, endpoint_id);
    match tokio::fs::remove_dir_all(&endpoint_home).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("removing kiro endpoint home for endpoint {}", endpoint_id)),
    }
}

async fn remove_kiro_endpoint_homes_for_runtime_roots(
    data_root: &Path,
    endpoint_id: &str,
) -> Result<()> {
    for runtime_root in container_runtime_data_roots(data_root).await {
        remove_kiro_endpoint_home_for_root(&runtime_root, endpoint_id).await?;
    }
    Ok(())
}

async fn remove_cline_endpoint_home_for_root(root: &Path, endpoint_id: &str) -> Result<()> {
    let endpoint_home = cline_endpoint_home(root, endpoint_id);
    match tokio::fs::remove_dir_all(&endpoint_home).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("removing cline endpoint home for endpoint {}", endpoint_id)),
    }
}

async fn remove_cline_endpoint_homes_for_runtime_roots(
    data_root: &Path,
    endpoint_id: &str,
) -> Result<()> {
    for runtime_root in container_runtime_data_roots(data_root).await {
        remove_cline_endpoint_home_for_root(&runtime_root, endpoint_id).await?;
    }
    Ok(())
}

fn normalize_provider_id(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        PROVIDER_CODEX => Some(PROVIDER_CODEX),
        PROVIDER_CLAUDE => Some(PROVIDER_CLAUDE),
        PROVIDER_GEMINI => Some(PROVIDER_GEMINI),
        PROVIDER_KIMI => Some(PROVIDER_KIMI),
        PROVIDER_QWEN => Some(PROVIDER_QWEN),
        PROVIDER_OPENCODE => Some(PROVIDER_OPENCODE),
        PROVIDER_MISTRAL => Some(PROVIDER_MISTRAL),
        PROVIDER_GOOSE => Some(PROVIDER_GOOSE),
        PROVIDER_CAGENT => Some(PROVIDER_CAGENT),
        PROVIDER_AMP => Some(PROVIDER_AMP),
        PROVIDER_DROID => Some(PROVIDER_DROID),
        PROVIDER_CODY => Some(PROVIDER_CODY),
        PROVIDER_CONTINUE => Some(PROVIDER_CONTINUE),
        PROVIDER_CLINE => Some(PROVIDER_CLINE),
        PROVIDER_OPENHANDS => Some(PROVIDER_OPENHANDS),
        PROVIDER_COPILOT => Some(PROVIDER_COPILOT),
        PROVIDER_KIRO => Some(PROVIDER_KIRO),
        PROVIDER_AUGGIE => Some(PROVIDER_AUGGIE),
        PROVIDER_PI => Some(PROVIDER_PI),
        PROVIDER_CURSOR => Some(PROVIDER_CURSOR),
        _ => None,
    }
}

fn provider_supports_harness_endpoint(canonical_provider_id: &str) -> bool {
    matches!(
        canonical_provider_id,
        PROVIDER_CODEX
            | PROVIDER_CLAUDE
            | PROVIDER_GEMINI
            | PROVIDER_KIMI
            | PROVIDER_QWEN
            | PROVIDER_OPENCODE
            | PROVIDER_MISTRAL
            | PROVIDER_GOOSE
            | PROVIDER_CAGENT
            | PROVIDER_AMP
            | PROVIDER_DROID
            | PROVIDER_CODY
            | PROVIDER_CONTINUE
            | PROVIDER_CLINE
            | PROVIDER_OPENHANDS
            | PROVIDER_COPILOT
            | PROVIDER_KIRO
            | PROVIDER_AUGGIE
            | PROVIDER_PI
    )
}

pub fn supports_harness_endpoint(provider_id: &str) -> bool {
    normalize_provider_id(provider_id).is_some_and(provider_supports_harness_endpoint)
}

pub fn default_shape_for_provider(provider_id: &str) -> Option<HarnessApiShape> {
    match normalize_provider_id(provider_id) {
        Some(PROVIDER_CODEX) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CLAUDE) => Some(HarnessApiShape::AnthropicMessages),
        Some(PROVIDER_GEMINI) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_KIMI) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_QWEN) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_OPENCODE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_MISTRAL) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_GOOSE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CAGENT) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_AMP) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_DROID) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CODY) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CONTINUE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_CLINE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_OPENHANDS) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_COPILOT) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_KIRO) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_AUGGIE) => Some(HarnessApiShape::OpenaiResponses),
        Some(PROVIDER_PI) => Some(HarnessApiShape::OpenaiResponses),
        _ => None,
    }
}

pub fn ensure_shape_compatible(provider_id: &str, shape: HarnessApiShape) -> Result<()> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    match canonical {
        PROVIDER_CODEX => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "codex requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        PROVIDER_CLAUDE => {
            if shape != HarnessApiShape::AnthropicMessages {
                anyhow::bail!(
                    "claude-crp requires api_shape=anthropic_messages; found {}",
                    shape.as_str()
                );
            }
        }
        PROVIDER_GEMINI => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "gemini requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        PROVIDER_KIMI => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "kimi requires api_shape=openai_responses; found {}",
                    shape.as_str()
                );
            }
        }
        PROVIDER_QWEN | PROVIDER_OPENCODE | PROVIDER_MISTRAL | PROVIDER_GOOSE | PROVIDER_CAGENT
        | PROVIDER_AMP | PROVIDER_DROID | PROVIDER_CODY | PROVIDER_CONTINUE | PROVIDER_CLINE
        | PROVIDER_OPENHANDS | PROVIDER_COPILOT | PROVIDER_KIRO | PROVIDER_AUGGIE | PROVIDER_PI => {
            if shape != HarnessApiShape::OpenaiResponses {
                anyhow::bail!(
                    "{} requires api_shape=openai_responses; found {}",
                    provider_id,
                    shape.as_str()
                );
            }
        }
        _ => anyhow::bail!("provider does not support harness endpoints: {provider_id}"),
    }
    Ok(())
}

async fn load_registry(data_root: &Path) -> Result<HarnessSourceRegistryInternal> {
    let path = registry_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing harness source registry {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(HarnessSourceRegistryInternal::default())
        }
        Err(err) => {
            Err(err).with_context(|| format!("reading harness source registry {}", path.display()))
        }
    }
}

async fn save_registry(data_root: &Path, registry: &HarnessSourceRegistryInternal) -> Result<()> {
    let path = registry_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(registry)?;
    let tmp_path = path.with_extension(format!("json.tmp.{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::rename(&tmp_path, &path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

fn normalize_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("base_url is required");
    }
    let parsed = Url::parse(trimmed).context("base_url must be a valid URL")?;
    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        anyhow::bail!("base_url must use http:// or https://");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn normalize_claude_anthropic_base_url(raw: &str) -> Result<String> {
    let normalized = normalize_base_url(raw)?;
    let mut parsed = Url::parse(&normalized).context("base_url must be a valid URL")?;
    let current_path = parsed.path().to_string();
    let lowered = current_path.to_ascii_lowercase();

    let suffixes = ["/v1/messages/count_tokens", "/v1/messages", "/v1"];
    let mut stripped_path: Option<String> = None;
    for suffix in suffixes {
        if lowered.ends_with(suffix) {
            let keep_len = current_path.len().saturating_sub(suffix.len());
            let prefix = current_path.get(..keep_len).unwrap_or_default();
            let trimmed = prefix.trim_end_matches('/');
            stripped_path = Some(if trimmed.is_empty() {
                "/".to_string()
            } else {
                trimmed.to_string()
            });
            break;
        }
    }

    if let Some(path) = stripped_path {
        parsed.set_path(&path);
    }

    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn provider_requires_endpoint_base_url(provider_id: &str) -> bool {
    matches!(
        provider_id,
        PROVIDER_CODEX
            | PROVIDER_CLAUDE
            | PROVIDER_KIMI
            | PROVIDER_QWEN
            | PROVIDER_OPENCODE
            | PROVIDER_MISTRAL
            | PROVIDER_GOOSE
            | PROVIDER_CAGENT
            | PROVIDER_CLINE
            | PROVIDER_OPENHANDS
    )
}

fn normalize_base_url_for_provider(provider_id: &str, raw: Option<&str>) -> Result<String> {
    match raw {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                if provider_requires_endpoint_base_url(provider_id) {
                    anyhow::bail!("base_url is required");
                }
                Ok(String::new())
            } else {
                if provider_id == PROVIDER_CLAUDE {
                    normalize_claude_anthropic_base_url(trimmed)
                } else {
                    normalize_base_url(trimmed)
                }
            }
        }
        None => {
            if provider_requires_endpoint_base_url(provider_id) {
                anyhow::bail!("base_url is required");
            }
            Ok(String::new())
        }
    }
}

fn normalize_auth_type_for_provider(provider_id: &str, raw: Option<&str>) -> Result<String> {
    match provider_id {
        PROVIDER_CODEX => Ok(CODEX_AUTH_TYPE_BEARER.to_string()),
        PROVIDER_CLAUDE => Ok(CLAUDE_AUTH_TYPE_API_KEY.to_string()),
        PROVIDER_GEMINI => {
            let normalized = raw
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(GEMINI_AUTH_TYPE_GEMINI_API_KEY)
                .to_ascii_lowercase();
            match normalized.as_str() {
                GEMINI_AUTH_TYPE_GEMINI_API_KEY
                | GEMINI_AUTH_TYPE_VERTEX_AI
                | CODEX_AUTH_TYPE_BEARER => Ok(normalized),
                _ => anyhow::bail!(
                    "auth_type '{}' is not supported for gemini (expected '{}' or '{}')",
                    normalized,
                    GEMINI_AUTH_TYPE_GEMINI_API_KEY,
                    GEMINI_AUTH_TYPE_VERTEX_AI
                ),
            }
        }
        _ => Ok(CODEX_AUTH_TYPE_BEARER.to_string()),
    }
}

fn endpoint_base_url_or_err(endpoint: &HarnessEndpointRecordInternal) -> Result<String> {
    let trimmed = endpoint.base_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!(
            "selected endpoint '{}' for {} is missing base_url",
            endpoint.name,
            endpoint.provider_id
        );
    }
    Ok(trimmed.to_string())
}

fn normalize_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("endpoint name is required");
    }
    Ok(trimmed.to_string())
}

fn normalize_manual_model_ids(input: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for raw in input {
        let normalized = raw.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            out.push(normalized.to_string());
        }
    }
    out
}

fn merge_endpoint_model_records(
    discovered: &[EndpointModelRecord],
    manual_model_ids: &[String],
) -> Vec<EndpointModelRecord> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged = Vec::new();
    for model in discovered {
        let id = model.id.trim();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_string()) {
            merged.push(EndpointModelRecord {
                id: id.to_string(),
                name: model.name.as_ref().map(|value| value.trim().to_string()),
            });
        }
    }
    for manual in manual_model_ids {
        let id = manual.trim();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_string()) {
            merged.push(EndpointModelRecord {
                id: id.to_string(),
                name: None,
            });
        }
    }
    merged
}

fn endpoint_models_url(base_url: &str) -> Result<String> {
    let mut normalized = normalize_base_url(base_url)?;
    normalized.push_str("/models");
    Ok(normalized)
}

fn infer_endpoint_model_provider_namespace(base_url: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok()?;
    let host = parsed.host_str()?.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }

    let mut labels = host.split('.').filter(|label| !label.is_empty());
    let candidate = labels
        .find(|label| !GENERIC_ENDPOINT_NAMESPACE_LABELS.contains(label))
        .or_else(|| host.split('.').find(|label| !label.is_empty()))?;

    let mut normalized = String::new();
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            normalized.push('_');
        }
    }
    let normalized = normalized.trim_matches('_').to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalize_namespaced_model_override(model: &str, endpoint_namespace: Option<&str>) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some(namespace) = endpoint_namespace
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return trimmed.to_string();
    };

    format!("{namespace}/{trimmed}")
}

fn truncate_discovery_error(raw: &str) -> String {
    const MAX: usize = 280;
    let collapsed = raw.replace(['\n', '\r'], " ").trim().to_string();
    if collapsed.len() <= MAX {
        return collapsed;
    }
    let mut end = MAX;
    while end > 0 && !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &collapsed[..end])
}

fn parse_openai_models_payload(payload: &serde_json::Value) -> Result<Vec<EndpointModelRecord>> {
    let data = payload
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("models payload missing array field 'data'"))?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut models = Vec::new();
    for entry in data {
        let Some(rec) = entry.as_object() else {
            continue;
        };
        let id = rec
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        if id.is_empty() {
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }
        let name = rec
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        models.push(EndpointModelRecord {
            id: id.to_string(),
            name,
        });
    }
    if models.is_empty() {
        anyhow::bail!("models payload did not include any model ids");
    }
    Ok(models)
}

async fn discover_openai_models(
    base_url: &str,
    auth_type: &str,
    api_key: &str,
) -> Result<Vec<EndpointModelRecord>> {
    let client = reqwest::Client::builder()
        .timeout(ENDPOINT_MODEL_DISCOVERY_TIMEOUT)
        .build()?;
    let url = endpoint_models_url(base_url)?;
    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json");
    match auth_type {
        GEMINI_AUTH_TYPE_GEMINI_API_KEY => {
            request = request.header("x-goog-api-key", api_key);
        }
        _ => {
            request = request.bearer_auth(api_key);
        }
    }
    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "model discovery failed with status {}: {}",
            status,
            truncate_discovery_error(&body)
        );
    }
    let payload: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| "model discovery response was not valid JSON")?;
    parse_openai_models_payload(&payload)
}

fn supports_model_discovery(endpoint: &HarnessEndpointRecordInternal) -> bool {
    endpoint.api_shape == HarnessApiShape::OpenaiResponses && !endpoint.base_url.trim().is_empty()
}

fn ensure_safe_endpoint_id(endpoint_id: &str) -> Result<()> {
    if endpoint_id.is_empty() {
        anyhow::bail!("endpoint_id is required");
    }
    if endpoint_id.len() > 128 {
        anyhow::bail!("endpoint_id must be 128 characters or fewer");
    }
    if !endpoint_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("endpoint_id may only contain ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn normalize_endpoint_id(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    ensure_safe_endpoint_id(trimmed)?;
    Ok(trimmed.to_string())
}

async fn write_endpoint_secret(data_root: &Path, secret_ref: &str, api_key: &str) -> Result<()> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        anyhow::bail!("api_key is required");
    }
    let dir = endpoint_secret_dir(data_root);
    tokio::fs::create_dir_all(&dir).await?;
    let path = endpoint_secret_path(data_root, secret_ref);
    let payload = serde_json::to_vec_pretty(&EndpointSecretEnvelope {
        version: SECRET_VERSION,
        api_key: trimmed.to_string(),
    })?;
    tokio::fs::write(&path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

fn parse_required_json_object(raw: &str, field_name: &str) -> Result<serde_json::Value> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).with_context(|| format!("{field_name} must be valid JSON"))?;
    if !parsed.is_object() {
        anyhow::bail!("{field_name} must be a JSON object");
    }
    Ok(parsed)
}

async fn read_endpoint_secret(data_root: &Path, secret_ref: &str) -> Result<String> {
    let path = endpoint_secret_path(data_root, secret_ref);
    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading endpoint secret {}", path.display()))?;
    let parsed: EndpointSecretEnvelope = serde_json::from_str(&raw)
        .with_context(|| format!("parsing endpoint secret {}", path.display()))?;
    let key = parsed.api_key.trim().to_string();
    if key.is_empty() {
        anyhow::bail!("endpoint secret has empty api_key");
    }
    Ok(key)
}

fn public_endpoint_from_internal(
    endpoint: &HarnessEndpointRecordInternal,
) -> HarnessEndpointRecord {
    let manual_model_ids = normalize_manual_model_ids(&endpoint.manual_model_ids);
    let model_catalog_models =
        merge_endpoint_model_records(&endpoint.model_catalog_models, &manual_model_ids);
    HarnessEndpointRecord {
        id: endpoint.id.clone(),
        provider_id: endpoint.provider_id.clone(),
        name: endpoint.name.clone(),
        base_url: if endpoint.base_url.trim().is_empty() {
            None
        } else {
            Some(endpoint.base_url.clone())
        },
        api_shape: endpoint.api_shape,
        auth_type: endpoint.auth_type.clone(),
        model_override: endpoint.model_override.clone(),
        created_at: endpoint.created_at,
        updated_at: endpoint.updated_at,
        last_verification_status: endpoint.last_verification_status,
        last_verification_at: endpoint.last_verification_at,
        last_error: endpoint.last_error.clone(),
        has_api_key: true,
        model_catalog_status: endpoint.model_catalog_status,
        model_catalog_fetched_at: endpoint.model_catalog_fetched_at,
        model_catalog_error: endpoint.model_catalog_error.clone(),
        model_catalog_models,
        manual_model_ids,
        model_catalog_source: endpoint.model_catalog_source.clone(),
    }
}

pub async fn get_provider_source_config(
    data_root: &Path,
    provider_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let endpoint_supported = provider_supports_harness_endpoint(canonical);
    let registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .get(canonical)
        .cloned()
        .unwrap_or_default();
    let mut selected_endpoint_id = provider.selected_endpoint_id;
    if !endpoint_supported {
        selected_endpoint_id = None;
    } else if provider.selected_source_kind == HarnessSourceKind::Endpoint {
        let exists = selected_endpoint_id
            .as_ref()
            .and_then(|id| provider.endpoints.iter().find(|ep| ep.id == *id))
            .is_some();
        if !exists {
            selected_endpoint_id = None;
        }
    }
    Ok(HarnessProviderSourceConfig {
        provider_id: canonical.to_string(),
        selected_source_kind: if endpoint_supported && selected_endpoint_id.is_some() {
            provider.selected_source_kind
        } else {
            HarnessSourceKind::Subscription
        },
        selected_endpoint_id,
        endpoints: if endpoint_supported {
            provider
                .endpoints
                .iter()
                .map(public_endpoint_from_internal)
                .collect()
        } else {
            Vec::new()
        },
    })
}

pub async fn find_provider_endpoint_import_match(
    data_root: &Path,
    provider_id: &str,
    base_url: Option<String>,
    api_shape: HarnessApiShape,
    auth_type: Option<String>,
    model_override: Option<String>,
    api_key: &str,
) -> Result<Option<HarnessEndpointImportMatch>> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    ensure_shape_compatible(canonical, api_shape)?;
    let normalized_base_url = normalize_base_url_for_provider(canonical, base_url.as_deref())?;
    let normalized_auth_type = normalize_auth_type_for_provider(canonical, auth_type.as_deref())?;
    let normalized_model_override = model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let registry = load_registry(data_root).await?;
    let Some(provider) = registry.providers.get(canonical) else {
        return Ok(None);
    };

    let mut config_match_endpoint_id: Option<String> = None;
    for endpoint in &provider.endpoints {
        if endpoint.base_url != normalized_base_url
            || endpoint.api_shape != api_shape
            || endpoint.auth_type != normalized_auth_type
        {
            continue;
        }
        let endpoint_model_override = endpoint
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if endpoint_model_override != normalized_model_override {
            continue;
        }
        if config_match_endpoint_id.is_none() {
            config_match_endpoint_id = Some(endpoint.id.clone());
        }
        if let Ok(existing_api_key) = read_endpoint_secret(data_root, &endpoint.secret_ref).await {
            if existing_api_key == api_key {
                return Ok(Some(HarnessEndpointImportMatch {
                    endpoint_id: endpoint.id.clone(),
                    kind: HarnessEndpointImportMatchKind::ExactCredentials,
                }));
            }
        }
    }

    Ok(
        config_match_endpoint_id.map(|endpoint_id| HarnessEndpointImportMatch {
            endpoint_id,
            kind: HarnessEndpointImportMatchKind::SameConfig,
        }),
    )
}

pub async fn upsert_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    input: HarnessEndpointUpsert,
) -> Result<HarnessEndpointRecord> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    let api_shape = input
        .api_shape
        .or_else(|| default_shape_for_provider(canonical))
        .ok_or_else(|| anyhow::anyhow!("api_shape is required"))?;
    ensure_shape_compatible(canonical, api_shape)?;

    let name = normalize_name(&input.name)?;
    let base_url = normalize_base_url_for_provider(canonical, input.base_url.as_deref())?;
    let model_override = input
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if canonical == PROVIDER_KIRO {
        if let Some(api_key) = input.api_key.as_ref() {
            let _ = parse_required_json_object(api_key, "api_key")?;
        }
    }

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let now = Utc::now();
    let endpoint_id = match input
        .endpoint_id
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => normalize_endpoint_id(&value)?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    ensure_safe_endpoint_id(&endpoint_id)?;

    let existing_index = provider
        .endpoints
        .iter()
        .position(|ep| ep.id == endpoint_id);
    let secret_ref = existing_index
        .and_then(|idx| provider.endpoints.get(idx).map(|ep| ep.secret_ref.clone()))
        .unwrap_or_else(|| format!("{}-{}.json", canonical, endpoint_id));

    if existing_index.is_none() && input.api_key.is_none() {
        anyhow::bail!("api_key is required when creating an endpoint");
    }

    if let Some(api_key) = input.api_key.as_ref() {
        write_endpoint_secret(data_root, &secret_ref, api_key).await?;
    }

    let auth_type = normalize_auth_type_for_provider(canonical, input.auth_type.as_deref())?;

    let mut next = HarnessEndpointRecordInternal {
        id: endpoint_id.clone(),
        provider_id: canonical.to_string(),
        name,
        base_url,
        api_shape,
        auth_type,
        model_override,
        created_at: now,
        updated_at: now,
        last_verification_status: HarnessEndpointVerificationStatus::Unknown,
        last_verification_at: None,
        last_error: None,
        model_catalog_status: EndpointModelCatalogStatus::Unknown,
        model_catalog_fetched_at: None,
        model_catalog_error: None,
        model_catalog_models: Vec::new(),
        manual_model_ids: Vec::new(),
        model_catalog_source: None,
        secret_ref,
    };

    if let Some(idx) = existing_index {
        if let Some(previous) = provider.endpoints.get(idx) {
            next.created_at = previous.created_at;
            next.model_catalog_status = previous.model_catalog_status;
            next.model_catalog_fetched_at = previous.model_catalog_fetched_at;
            next.model_catalog_error = previous.model_catalog_error.clone();
            next.model_catalog_models = previous.model_catalog_models.clone();
            next.manual_model_ids = previous.manual_model_ids.clone();
            next.model_catalog_source = previous.model_catalog_source.clone();
        }
        provider.endpoints[idx] = next.clone();
    } else {
        provider.endpoints.push(next.clone());
    }

    save_registry(data_root, &registry).await?;
    Ok(public_endpoint_from_internal(&next))
}

pub async fn delete_provider_endpoint(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    if !provider_supports_harness_endpoint(canonical) {
        anyhow::bail!("provider does not support harness endpoints: {provider_id}");
    }
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    let before = provider.endpoints.len();
    let removed: Vec<(String, String)> = provider
        .endpoints
        .iter()
        .filter(|ep| ep.id == endpoint_id)
        .map(|ep| (ep.id.clone(), ep.secret_ref.clone()))
        .collect();
    provider.endpoints.retain(|ep| ep.id != endpoint_id);

    if provider.selected_endpoint_id.as_deref() == Some(endpoint_id) {
        provider.selected_source_kind = HarnessSourceKind::Subscription;
        provider.selected_endpoint_id = None;
    }

    if before != provider.endpoints.len() {
        for (removed_endpoint_id, secret_ref) in removed {
            let secret_path = endpoint_secret_path(data_root, &secret_ref);
            match tokio::fs::remove_file(&secret_path).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("removing endpoint secret {}", secret_path.display())
                    });
                }
            }

            if canonical == PROVIDER_CODEX {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                let endpoint_home = codex_endpoint_home(data_root, &removed_endpoint_id);
                match tokio::fs::remove_dir_all(&endpoint_home).await {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "removing codex endpoint home for endpoint {}",
                                removed_endpoint_id
                            )
                        });
                    }
                }
            } else if canonical == PROVIDER_QWEN {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                let endpoint_home = qwen_endpoint_home(data_root, &removed_endpoint_id);
                match tokio::fs::remove_dir_all(&endpoint_home).await {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "removing qwen endpoint home for endpoint {}",
                                removed_endpoint_id
                            )
                        });
                    }
                }
            } else if canonical == PROVIDER_KIRO {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                remove_kiro_endpoint_home_for_root(data_root, &removed_endpoint_id).await?;
                remove_kiro_endpoint_homes_for_runtime_roots(data_root, &removed_endpoint_id)
                    .await?;
            } else if canonical == PROVIDER_CLINE {
                ensure_safe_endpoint_id(&removed_endpoint_id)?;
                remove_cline_endpoint_home_for_root(data_root, &removed_endpoint_id).await?;
                remove_cline_endpoint_homes_for_runtime_roots(data_root, &removed_endpoint_id)
                    .await?;
            }
        }
        save_registry(data_root, &registry).await?;
    }

    get_provider_source_config(data_root, canonical).await
}

pub async fn set_provider_source_selection(
    data_root: &Path,
    provider_id: &str,
    source_kind: HarnessSourceKind,
    endpoint_id: Option<String>,
) -> Result<HarnessProviderSourceConfig> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .entry(canonical.to_string())
        .or_insert_with(HarnessProviderConfigInternal::default);

    match source_kind {
        HarnessSourceKind::Subscription => {
            provider.selected_source_kind = HarnessSourceKind::Subscription;
            provider.selected_endpoint_id = None;
        }
        HarnessSourceKind::Endpoint => {
            if !provider_supports_harness_endpoint(canonical) {
                anyhow::bail!("provider does not support harness endpoints: {provider_id}");
            }
            let endpoint_id = endpoint_id
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("endpoint_id is required for endpoint source"))?;
            let exists = provider.endpoints.iter().any(|ep| ep.id == endpoint_id);
            if !exists {
                anyhow::bail!("unknown endpoint_id: {}", endpoint_id);
            }
            provider.selected_source_kind = HarnessSourceKind::Endpoint;
            provider.selected_endpoint_id = Some(endpoint_id);
        }
    }

    save_registry(data_root, &registry).await?;
    get_provider_source_config(data_root, canonical).await
}

pub async fn mark_endpoint_verification(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
    status: HarnessEndpointVerificationStatus,
    error: Option<String>,
) -> Result<()> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let Some(provider) = registry.providers.get_mut(canonical) else {
        return Ok(());
    };
    let Some(endpoint) = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
    else {
        return Ok(());
    };
    endpoint.last_verification_status = status;
    endpoint.last_verification_at = Some(Utc::now());
    endpoint.last_error = error;
    endpoint.updated_at = Utc::now();
    save_registry(data_root, &registry).await
}

pub fn endpoint_model_catalog_ttl() -> Duration {
    ENDPOINT_MODEL_CATALOG_TTL
}

pub fn endpoint_model_catalog_is_stale(
    endpoint: &HarnessEndpointRecord,
    now: DateTime<Utc>,
) -> bool {
    if endpoint.model_catalog_status == EndpointModelCatalogStatus::Unknown {
        return true;
    }
    if endpoint.model_catalog_status == EndpointModelCatalogStatus::Error {
        return true;
    }
    let Some(fetched_at) = endpoint.model_catalog_fetched_at else {
        return endpoint.model_catalog_status != EndpointModelCatalogStatus::ManualOnly;
    };
    let age = now.signed_duration_since(fetched_at);
    age > chrono::Duration::from_std(ENDPOINT_MODEL_CATALOG_TTL)
        .unwrap_or_else(|_| chrono::Duration::hours(24))
}

pub async fn set_provider_endpoint_manual_models(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
    manual_model_ids: Vec<String>,
) -> Result<HarnessEndpointRecord> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;
    let normalized_manual = normalize_manual_model_ids(&manual_model_ids);
    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .get_mut(canonical)
        .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {}", canonical))?;
    let endpoint = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
        .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {}", endpoint_id))?;

    endpoint.manual_model_ids = normalized_manual.clone();
    endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
        if endpoint.model_catalog_models.is_empty() {
            None
        } else {
            Some("discovered".to_string())
        }
    } else if endpoint.model_catalog_models.is_empty() {
        Some("manual".to_string())
    } else {
        Some("mixed".to_string())
    };
    endpoint.model_catalog_status = if endpoint.manual_model_ids.is_empty() {
        if endpoint.model_catalog_models.is_empty() {
            EndpointModelCatalogStatus::Unknown
        } else {
            EndpointModelCatalogStatus::Ready
        }
    } else if endpoint.model_catalog_models.is_empty() {
        EndpointModelCatalogStatus::ManualOnly
    } else {
        EndpointModelCatalogStatus::Ready
    };
    endpoint.updated_at = Utc::now();

    let public = public_endpoint_from_internal(endpoint);
    save_registry(data_root, &registry).await?;
    Ok(public)
}

pub async fn refresh_provider_endpoint_model_catalog(
    data_root: &Path,
    provider_id: &str,
    endpoint_id: &str,
) -> Result<HarnessEndpointRecord> {
    let canonical = normalize_provider_id(provider_id).ok_or_else(|| {
        anyhow::anyhow!("provider does not support harness endpoints: {provider_id}")
    })?;

    let endpoint_snapshot = {
        let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
        let registry = load_registry(data_root).await?;
        let provider = registry
            .providers
            .get(canonical)
            .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {}", canonical))?;
        provider
            .endpoints
            .iter()
            .find(|ep| ep.id == endpoint_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {}", endpoint_id))?
    };
    let api_key = read_endpoint_secret(data_root, &endpoint_snapshot.secret_ref).await?;
    let discovery_result = if supports_model_discovery(&endpoint_snapshot) {
        discover_openai_models(
            &endpoint_snapshot.base_url,
            &endpoint_snapshot.auth_type,
            &api_key,
        )
        .await
    } else {
        Err(anyhow::anyhow!(
            "model discovery is unsupported for provider '{}' with api_shape '{}' and base_url '{}'",
            canonical,
            endpoint_snapshot.api_shape.as_str(),
            endpoint_snapshot.base_url
        ))
    };

    let _registry_write_guard = REGISTRY_WRITE_LOCK.lock().await;
    let mut registry = load_registry(data_root).await?;
    let provider = registry
        .providers
        .get_mut(canonical)
        .ok_or_else(|| anyhow::anyhow!("unknown provider endpoint config for {}", canonical))?;
    let endpoint = provider
        .endpoints
        .iter_mut()
        .find(|ep| ep.id == endpoint_id)
        .ok_or_else(|| anyhow::anyhow!("unknown endpoint_id: {}", endpoint_id))?;

    endpoint.updated_at = Utc::now();
    match discovery_result {
        Ok(discovered_models) => {
            endpoint.model_catalog_models = discovered_models;
            endpoint.model_catalog_fetched_at = Some(Utc::now());
            endpoint.model_catalog_error = None;
            endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
                Some("discovered".to_string())
            } else {
                Some("mixed".to_string())
            };
            endpoint.model_catalog_status = if endpoint.model_catalog_models.is_empty() {
                if endpoint.manual_model_ids.is_empty() {
                    EndpointModelCatalogStatus::Error
                } else {
                    EndpointModelCatalogStatus::ManualOnly
                }
            } else {
                EndpointModelCatalogStatus::Ready
            };
        }
        Err(err) => {
            endpoint.model_catalog_error = Some(truncate_discovery_error(&err.to_string()));
            endpoint.model_catalog_source = if endpoint.manual_model_ids.is_empty() {
                if endpoint.model_catalog_models.is_empty() {
                    None
                } else {
                    Some("discovered".to_string())
                }
            } else if endpoint.model_catalog_models.is_empty() {
                Some("manual".to_string())
            } else {
                Some("mixed".to_string())
            };
            endpoint.model_catalog_status = if endpoint.manual_model_ids.is_empty() {
                if endpoint.model_catalog_models.is_empty() {
                    EndpointModelCatalogStatus::Error
                } else {
                    EndpointModelCatalogStatus::Ready
                }
            } else if endpoint.model_catalog_models.is_empty() {
                EndpointModelCatalogStatus::ManualOnly
            } else {
                EndpointModelCatalogStatus::Ready
            };
        }
    }

    let public = public_endpoint_from_internal(endpoint);
    save_registry(data_root, &registry).await?;
    Ok(public)
}

async fn prepare_codex_home_with_api_key(codex_home: &Path, api_key: &str) -> Result<()> {
    tokio::fs::create_dir_all(codex_home).await?;
    let auth_path = codex_home.join("auth.json");
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "OPENAI_API_KEY": api_key,
    }))?;
    tokio::fs::write(&auth_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(auth_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

async fn prepare_qwen_home_with_openai_settings(qwen_home: &Path) -> Result<()> {
    let qwen_config = qwen_home.join(".qwen");
    tokio::fs::create_dir_all(&qwen_config).await?;
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "$version": 2,
        "security": {
            "auth": {
                "selectedType": "openai"
            }
        }
    }))?;
    tokio::fs::write(qwen_config.join("settings.json"), payload).await?;
    Ok(())
}

async fn prepare_kiro_home_with_auth_token_json(
    kiro_home: &Path,
    auth_token_json: &str,
) -> Result<()> {
    let auth_token = parse_required_json_object(auth_token_json, "api_key")?;
    tokio::fs::create_dir_all(kiro_home).await?;
    let token_path = kiro_home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
    if let Some(parent) = token_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_vec_pretty(&auth_token)?;
    tokio::fs::write(&token_path, payload).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            tokio::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::create_dir_all(kiro_home.join(".config")).await?;
    tokio::fs::create_dir_all(kiro_home.join(".cache")).await?;
    Ok(())
}

fn provider_store<'a>(
    registry: &'a HarnessSourceRegistryInternal,
    provider_id: &str,
) -> Option<&'a HarnessProviderConfigInternal> {
    registry.providers.get(provider_id)
}

async fn resolve_internal(
    data_root: &Path,
    provider_id: &str,
    require_verified_endpoint: bool,
    runtime_data_root: Option<&Path>,
) -> Result<ResolvedHarnessSource> {
    let canonical = match normalize_provider_id(provider_id) {
        Some(id) => id,
        None => {
            return Ok(ResolvedHarnessSource {
                source_kind: HarnessSourceKind::Subscription,
                endpoint: None,
                env: HashMap::new(),
            });
        }
    };

    let registry = load_registry(data_root).await?;
    let provider = provider_store(&registry, canonical)
        .cloned()
        .unwrap_or_default();

    if provider.selected_source_kind != HarnessSourceKind::Endpoint {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: subscription_env_for_provider(canonical, data_root, runtime_data_root),
        });
    }

    if !provider_supports_harness_endpoint(canonical) {
        return Ok(ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: subscription_env_for_provider(canonical, data_root, runtime_data_root),
        });
    }

    let endpoint_id = provider
        .selected_endpoint_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no endpoint selected for provider {}", canonical))?;
    let endpoint = provider
        .endpoints
        .iter()
        .find(|ep| ep.id == endpoint_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("selected endpoint not found for provider {}", canonical))?;

    if require_verified_endpoint
        && endpoint.last_verification_status != HarnessEndpointVerificationStatus::Valid
    {
        anyhow::bail!(
            "selected endpoint '{}' for {} is not verified; verify it in Settings before running",
            endpoint.name,
            canonical
        );
    }

    let api_key = read_endpoint_secret(data_root, &endpoint.secret_ref).await?;
    let mut env = HashMap::new();

    match canonical {
        PROVIDER_CODEX => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let codex_home = codex_endpoint_home(data_root, &endpoint.id);
            prepare_codex_home_with_api_key(&codex_home, &api_key).await?;
            env.insert(
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().to_string(),
            );
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
        }
        PROVIDER_CLAUDE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("ANTHROPIC_API_KEY".to_string(), api_key);
            env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);
        }
        PROVIDER_GEMINI => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            match endpoint.auth_type.as_str() {
                GEMINI_AUTH_TYPE_VERTEX_AI => {
                    env.insert("GOOGLE_API_KEY".to_string(), api_key);
                    env.insert("GOOGLE_GENAI_USE_VERTEXAI".to_string(), "true".to_string());
                }
                GEMINI_AUTH_TYPE_GEMINI_API_KEY => {
                    env.insert("GEMINI_API_KEY".to_string(), api_key);
                }
                _ => {
                    // Legacy compatibility for previously stored OpenAI-compatible Gemini endpoints.
                    let base_url = endpoint_base_url_or_err(&endpoint)?;
                    env.insert("OPENAI_API_KEY".to_string(), api_key);
                    env.insert("OPENAI_BASE_URL".to_string(), base_url);
                }
            }
        }
        PROVIDER_KIMI => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("KIMI_API_KEY".to_string(), api_key);
            env.insert("KIMI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("KIMI_MODEL_NAME".to_string(), model);
            }
        }
        PROVIDER_QWEN => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let qwen_home_root = runtime_data_root.unwrap_or(data_root);
            let qwen_home = qwen_endpoint_home(qwen_home_root, &endpoint.id);
            prepare_qwen_home_with_openai_settings(&qwen_home).await?;
            env.insert("HOME".to_string(), qwen_home.to_string_lossy().to_string());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_CLINE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let cline_home_root = runtime_data_root.unwrap_or(data_root);
            let cline_home = cline_endpoint_home(cline_home_root, &endpoint.id);
            tokio::fs::create_dir_all(&cline_home)
                .await
                .with_context(|| {
                    format!(
                        "creating cline endpoint home {}",
                        cline_home.to_string_lossy()
                    )
                })?;
            let cline_home_str = cline_home.to_string_lossy().to_string();
            env.insert("CLINE_DIR".to_string(), cline_home_str.clone());
            env.insert("HOME".to_string(), cline_home_str);
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_CAGENT => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_OPENCODE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            let provider_namespace = infer_endpoint_model_provider_namespace(&base_url)
                .unwrap_or_else(|| "endpoint".to_string());
            env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
            env.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
            if provider_namespace == "openrouter" {
                env.insert("OPENROUTER_API_KEY".to_string(), api_key.clone());
                env.insert("OPENROUTER_BASE_URL".to_string(), base_url.clone());
            }

            let mut provider_config = serde_json::Map::new();
            provider_config.insert(
                provider_namespace.clone(),
                serde_json::json!({
                    "options": {
                        "baseURL": base_url,
                        "apiKey": api_key,
                    }
                }),
            );
            let mut root = serde_json::Map::new();
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                root.insert(
                    "model".to_string(),
                    serde_json::Value::String(normalize_namespaced_model_override(
                        &model,
                        Some(provider_namespace.as_str()),
                    )),
                );
            }
            root.insert(
                "provider".to_string(),
                serde_json::Value::Object(provider_config),
            );
            env.insert(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                serde_json::Value::Object(root).to_string(),
            );
        }
        PROVIDER_GOOSE => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            env.insert("GOOSE_PROVIDER".to_string(), "openai".to_string());
            env.insert("GOOSE_DISABLE_KEYRING".to_string(), "1".to_string());
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("GOOSE_MODEL".to_string(), model);
            }
        }
        PROVIDER_MISTRAL => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("MISTRAL_API_KEY".to_string(), api_key.clone());
            env.insert("MISTRAL_BASE_URL".to_string(), base_url.clone());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
        }
        PROVIDER_AMP => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("AMP_API_KEY".to_string(), api_key);
        }
        PROVIDER_DROID => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("FACTORY_API_KEY".to_string(), api_key);
        }
        PROVIDER_CODY => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("SRC_ACCESS_TOKEN".to_string(), api_key);
            let base_url = endpoint.base_url.trim().to_string();
            if !base_url.is_empty() {
                env.insert("SRC_ENDPOINT".to_string(), base_url);
            }
        }
        PROVIDER_CONTINUE => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("CONTINUE_API_KEY".to_string(), api_key);
        }
        PROVIDER_OPENHANDS => {
            let base_url = endpoint_base_url_or_err(&endpoint)?;
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("LLM_API_KEY".to_string(), api_key.clone());
            env.insert("LLM_BASE_URL".to_string(), base_url.clone());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            env.insert("OPENAI_BASE_URL".to_string(), base_url);
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("LLM_MODEL".to_string(), model.clone());
                env.insert("OPENAI_MODEL".to_string(), model);
            }
        }
        PROVIDER_COPILOT => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("GH_TOKEN".to_string(), api_key.clone());
            env.insert("GITHUB_TOKEN".to_string(), api_key);
        }
        PROVIDER_PI => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("PI_ACP_PROVIDER".to_string(), "openai".to_string());
            env.insert("OPENAI_API_KEY".to_string(), api_key);
            let base_url = endpoint.base_url.trim().to_string();
            if !base_url.is_empty() {
                env.insert("OPENAI_BASE_URL".to_string(), base_url);
            }
            if let Some(model) = endpoint
                .model_override
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                env.insert("PI_ACP_MODEL".to_string(), model);
            }
        }
        PROVIDER_KIRO => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            ensure_safe_endpoint_id(&endpoint.id)?;
            let kiro_home_root = runtime_data_root.unwrap_or(data_root);
            let kiro_home = kiro_endpoint_home(kiro_home_root, &endpoint.id);
            prepare_kiro_home_with_auth_token_json(&kiro_home, &api_key).await?;
            env.insert("HOME".to_string(), kiro_home.to_string_lossy().to_string());
            env.insert(
                "XDG_CONFIG_HOME".to_string(),
                kiro_home.join(".config").to_string_lossy().to_string(),
            );
            env.insert(
                "XDG_CACHE_HOME".to_string(),
                kiro_home.join(".cache").to_string_lossy().to_string(),
            );
        }
        PROVIDER_AUGGIE => {
            ensure_shape_compatible(canonical, endpoint.api_shape)?;
            env.insert("AUGMENT_SESSION_AUTH".to_string(), api_key.clone());
            env.insert("AUGMENT_API_TOKEN".to_string(), api_key);
        }
        _ => {}
    }

    let public = public_endpoint_from_internal(&endpoint);
    Ok(ResolvedHarnessSource {
        source_kind: HarnessSourceKind::Endpoint,
        endpoint: Some(public),
        env,
    })
}

pub async fn resolve_provider_source_for_probe(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_internal(data_root, provider_id, false, None).await
}

pub async fn resolve_provider_source_for_run(
    data_root: &Path,
    provider_id: &str,
) -> Result<ResolvedHarnessSource> {
    resolve_provider_source_for_run_with_runtime_root(data_root, provider_id, None).await
}

pub async fn resolve_provider_source_for_run_with_runtime_root(
    data_root: &Path,
    provider_id: &str,
    runtime_data_root: Option<&Path>,
) -> Result<ResolvedHarnessSource> {
    let require_verified_endpoint = matches!(
        normalize_provider_id(provider_id),
        Some(PROVIDER_CODEX) | Some(PROVIDER_CLAUDE)
    );
    resolve_internal(
        data_root,
        provider_id,
        require_verified_endpoint,
        runtime_data_root,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_supports_endpoint_mode_but_cursor_does_not() {
        assert!(supports_harness_endpoint(PROVIDER_CLAUDE));
        assert!(!supports_harness_endpoint(PROVIDER_CURSOR));
        assert!(supports_harness_endpoint(PROVIDER_CODEX));
    }

    #[tokio::test]
    async fn cursor_source_config_defaults_to_subscription_without_endpoints() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = get_provider_source_config(root.path(), PROVIDER_CURSOR)
            .await
            .expect("config");
        assert_eq!(cfg.selected_source_kind, HarnessSourceKind::Subscription);
        assert!(cfg.selected_endpoint_id.is_none());
        assert!(cfg.endpoints.is_empty());
    }

    #[tokio::test]
    async fn cursor_rejects_endpoint_source_selection() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = set_provider_source_selection(
            root.path(),
            PROVIDER_CURSOR,
            HarnessSourceKind::Endpoint,
            Some("ep-1".to_string()),
        )
        .await
        .expect_err("cursor endpoint mode should be rejected");
        assert!(err
            .to_string()
            .contains("provider does not support harness endpoints"));
    }

    #[test]
    fn parse_openai_models_payload_extracts_unique_ids() {
        let payload = serde_json::json!({
            "data": [
                { "id": "openai/gpt-5.2", "name": "GPT-5.2" },
                { "id": "openai/gpt-5.2" },
                { "id": "openai/gpt-4.1" },
                { "id": "" },
                {}
            ]
        });
        let models = parse_openai_models_payload(&payload).expect("models should parse");
        assert_eq!(
            models,
            vec![
                EndpointModelRecord {
                    id: "openai/gpt-5.2".to_string(),
                    name: Some("GPT-5.2".to_string()),
                },
                EndpointModelRecord {
                    id: "openai/gpt-4.1".to_string(),
                    name: None,
                },
            ]
        );
    }

    #[test]
    fn infer_endpoint_model_provider_namespace_prefers_non_generic_host_label() {
        assert_eq!(
            infer_endpoint_model_provider_namespace("https://openrouter.ai/api/v1"),
            Some("openrouter".to_string())
        );
        assert_eq!(
            infer_endpoint_model_provider_namespace("https://api.myawesomeprovider.example/v1"),
            Some("myawesomeprovider".to_string())
        );
    }

    #[test]
    fn normalize_namespaced_model_override_always_prefixes_namespace() {
        assert_eq!(
            normalize_namespaced_model_override("openai/gpt-5.2-codex", Some("openrouter")),
            "openrouter/openai/gpt-5.2-codex"
        );
        assert_eq!(
            normalize_namespaced_model_override(
                "openrouter/openai/gpt-5.2-codex",
                Some("openrouter"),
            ),
            "openrouter/openrouter/openai/gpt-5.2-codex"
        );
        assert_eq!(
            normalize_namespaced_model_override(
                "myawesomeprovider/openai/gpt-5.2-codex",
                Some("openrouter"),
            ),
            "openrouter/myawesomeprovider/openai/gpt-5.2-codex"
        );
    }

    #[test]
    fn parse_openai_models_payload_requires_data_array() {
        let payload = serde_json::json!({
            "models": []
        });
        let err = parse_openai_models_payload(&payload).expect_err("missing data should error");
        assert!(err
            .to_string()
            .contains("models payload missing array field 'data'"));
    }

    #[test]
    fn normalize_manual_model_ids_deduplicates_and_trims() {
        let input = vec![
            " openai/gpt-5.2 ".to_string(),
            "".to_string(),
            "openai/gpt-5.2".to_string(),
            "anthropic/claude-sonnet-4.5".to_string(),
        ];
        assert_eq!(
            normalize_manual_model_ids(&input),
            vec![
                "openai/gpt-5.2".to_string(),
                "anthropic/claude-sonnet-4.5".to_string(),
            ]
        );
    }

    #[test]
    fn truncate_discovery_error_preserves_utf8_boundaries() {
        let raw = format!("error: {}", "界".repeat(400));
        let truncated = truncate_discovery_error(&raw);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 283);
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[test]
    fn endpoint_catalog_stale_logic_handles_status_and_age() {
        let now = Utc::now();
        let mut endpoint = HarnessEndpointRecord {
            id: "ep-1".to_string(),
            provider_id: PROVIDER_CODEX.to_string(),
            name: "OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: HarnessApiShape::OpenaiResponses,
            auth_type: CODEX_AUTH_TYPE_BEARER.to_string(),
            model_override: None,
            created_at: now,
            updated_at: now,
            last_verification_status: HarnessEndpointVerificationStatus::Unknown,
            last_verification_at: None,
            last_error: None,
            has_api_key: true,
            model_catalog_status: EndpointModelCatalogStatus::Unknown,
            model_catalog_fetched_at: None,
            model_catalog_error: None,
            model_catalog_models: Vec::new(),
            manual_model_ids: Vec::new(),
            model_catalog_source: None,
        };
        assert!(endpoint_model_catalog_is_stale(&endpoint, now));

        endpoint.model_catalog_status = EndpointModelCatalogStatus::ManualOnly;
        assert!(!endpoint_model_catalog_is_stale(&endpoint, now));

        endpoint.model_catalog_status = EndpointModelCatalogStatus::Ready;
        endpoint.model_catalog_fetched_at = Some(now - chrono::Duration::hours(1));
        assert!(!endpoint_model_catalog_is_stale(&endpoint, now));

        endpoint.model_catalog_fetched_at = Some(now - chrono::Duration::hours(30));
        assert!(endpoint_model_catalog_is_stale(&endpoint, now));
    }

    #[tokio::test]
    async fn defaults_to_subscription_without_registry() {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = get_provider_source_config(root.path(), PROVIDER_CODEX)
            .await
            .expect("config");
        assert_eq!(cfg.selected_source_kind, HarnessSourceKind::Subscription);
        assert!(cfg.selected_endpoint_id.is_none());
        assert!(cfg.endpoints.is_empty());
    }

    #[tokio::test]
    async fn amp_subscription_sets_persistent_home_env() {
        let root = tempfile::tempdir().expect("tempdir");
        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_AMP)
            .await
            .expect("resolved");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Subscription);
        let expected_home = root.path().join("providers").join("amp").join("home");
        assert_eq!(
            resolved.env.get("HOME"),
            Some(&expected_home.to_string_lossy().to_string())
        );
        assert_eq!(
            resolved.env.get("XDG_CONFIG_HOME"),
            Some(&expected_home.join(".config").to_string_lossy().to_string())
        );
        assert_eq!(
            resolved.env.get("XDG_CACHE_HOME"),
            Some(&expected_home.join(".cache").to_string_lossy().to_string())
        );
    }

    #[tokio::test]
    async fn invalid_registry_json_returns_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = registry_path(root.path());
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&path, b"{not-json")
            .await
            .expect("write invalid");

        let err = get_provider_source_config(root.path(), PROVIDER_CODEX)
            .await
            .expect_err("invalid registry should fail");
        assert!(err.to_string().contains("parsing harness source registry"));
    }

    #[tokio::test]
    async fn codex_endpoint_requires_verify_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: None,
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CODEX,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let err = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
            .await
            .expect_err("expected verify gate error");
        assert!(err.to_string().contains("not verified"));

        mark_endpoint_verification(
            root.path(),
            PROVIDER_CODEX,
            &endpoint.id,
            HarnessEndpointVerificationStatus::Valid,
            None,
        )
        .await
        .expect("mark verified");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CODEX)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert!(resolved.env.contains_key("CODEX_HOME"));
        assert_eq!(
            resolved.env.get("OPENAI_BASE_URL"),
            Some(&"https://openrouter.ai/api/v1".to_string())
        );
    }

    #[tokio::test]
    async fn claude_endpoint_projects_anthropic_env_and_requires_verify_for_run() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CLAUDE,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Anthropic".to_string(),
                base_url: Some("https://api.anthropic.com/v1".to_string()),
                api_shape: Some(HarnessApiShape::AnthropicMessages),
                auth_type: None,
                model_override: None,
                api_key: Some("sk-ant-api".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CLAUDE,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let probe = resolve_provider_source_for_probe(root.path(), PROVIDER_CLAUDE)
            .await
            .expect("resolve probe");
        assert_eq!(
            probe.env.get("ANTHROPIC_API_KEY"),
            Some(&"sk-ant-api".to_string())
        );
        assert_eq!(
            probe.env.get("ANTHROPIC_BASE_URL"),
            Some(&"https://api.anthropic.com".to_string())
        );

        let run_err = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
            .await
            .expect_err("expected verify gate error");
        assert!(run_err.to_string().contains("not verified"));

        mark_endpoint_verification(
            root.path(),
            PROVIDER_CLAUDE,
            &endpoint.id,
            HarnessEndpointVerificationStatus::Valid,
            None,
        )
        .await
        .expect("mark verified");

        let run = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
            .await
            .expect("resolve run");
        assert_eq!(
            run.env.get("ANTHROPIC_BASE_URL"),
            Some(&"https://api.anthropic.com".to_string())
        );
    }

    #[tokio::test]
    async fn claude_endpoint_openrouter_v1_base_url_is_normalized_for_anthropic_shape() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CLAUDE,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::AnthropicMessages),
                auth_type: None,
                model_override: Some("anthropic/claude-opus-4.6".to_string()),
                api_key: Some("sk-or-v1".to_string()),
            },
        )
        .await
        .expect("upsert");

        assert_eq!(
            endpoint.base_url,
            Some("https://openrouter.ai/api".to_string())
        );

        set_provider_source_selection(
            root.path(),
            PROVIDER_CLAUDE,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        mark_endpoint_verification(
            root.path(),
            PROVIDER_CLAUDE,
            &endpoint.id,
            HarnessEndpointVerificationStatus::Valid,
            None,
        )
        .await
        .expect("mark verified");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_CLAUDE)
            .await
            .expect("resolve run");
        assert_eq!(
            resolved.env.get("ANTHROPIC_BASE_URL"),
            Some(&"https://openrouter.ai/api".to_string())
        );
    }

    #[tokio::test]
    async fn gemini_endpoint_does_not_require_verify_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_GEMINI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Gemini Key".to_string(),
                base_url: None,
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: Some(GEMINI_AUTH_TYPE_GEMINI_API_KEY.to_string()),
                model_override: None,
                api_key: Some("gemini-key".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_GEMINI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_GEMINI)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(endpoint.base_url, None);
        assert_eq!(
            resolved.env.get("GEMINI_API_KEY"),
            Some(&"gemini-key".to_string())
        );
        assert!(!resolved.env.contains_key("OPENAI_API_KEY"));
        assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
    }

    #[tokio::test]
    async fn gemini_vertex_endpoint_projects_vertex_env_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_GEMINI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Gemini Vertex".to_string(),
                base_url: None,
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: Some(GEMINI_AUTH_TYPE_VERTEX_AI.to_string()),
                model_override: None,
                api_key: Some("vertex-key".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_GEMINI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_GEMINI)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(
            resolved.env.get("GOOGLE_API_KEY"),
            Some(&"vertex-key".to_string())
        );
        assert_eq!(
            resolved.env.get("GOOGLE_GENAI_USE_VERTEXAI"),
            Some(&"true".to_string())
        );
        assert!(!resolved.env.contains_key("OPENAI_API_KEY"));
        assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
    }

    #[tokio::test]
    async fn gemini_endpoint_rejects_unknown_auth_type() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = upsert_provider_endpoint(
            root.path(),
            PROVIDER_GEMINI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Gemini Invalid".to_string(),
                base_url: None,
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: Some("invalid".to_string()),
                model_override: None,
                api_key: Some("gemini-key".to_string()),
            },
        )
        .await
        .expect_err("upsert should fail");
        assert!(err
            .to_string()
            .contains("auth_type 'invalid' is not supported"));
    }

    #[tokio::test]
    async fn kimi_endpoint_projects_kimi_env_for_run_resolution() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIMI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kimi Key".to_string(),
                base_url: Some("https://api.moonshot.ai/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some("kimi-k2".to_string()),
                api_key: Some("kimi-key".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIMI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_KIMI)
            .await
            .expect("resolve run");
        assert_eq!(resolved.source_kind, HarnessSourceKind::Endpoint);
        assert_eq!(
            resolved.env.get("KIMI_API_KEY"),
            Some(&"kimi-key".to_string())
        );
        assert_eq!(
            resolved.env.get("KIMI_BASE_URL"),
            Some(&"https://api.moonshot.ai/v1".to_string())
        );
        assert_eq!(
            resolved.env.get("KIMI_MODEL_NAME"),
            Some(&"kimi-k2".to_string())
        );
    }

    #[tokio::test]
    async fn qwen_model_override_is_opaque_and_opencode_uses_endpoint_namespace() {
        let root = tempfile::tempdir().expect("tempdir");
        let base_url = "https://api.myawesomeprovider.example/v1";
        let model_override = "openai/gpt-5.2-codex";

        let qwen_endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_QWEN,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Qwen custom".to_string(),
                base_url: Some(base_url.to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some(model_override.to_string()),
                api_key: Some("qwen-key".to_string()),
            },
        )
        .await
        .expect("upsert qwen endpoint");
        set_provider_source_selection(
            root.path(),
            PROVIDER_QWEN,
            HarnessSourceKind::Endpoint,
            Some(qwen_endpoint.id.clone()),
        )
        .await
        .expect("select qwen endpoint");
        let qwen_resolved = resolve_provider_source_for_run(root.path(), PROVIDER_QWEN)
            .await
            .expect("resolve qwen");
        assert_eq!(
            qwen_resolved.env.get("OPENAI_MODEL"),
            Some(&"openai/gpt-5.2-codex".to_string())
        );
        let qwen_home = PathBuf::from(
            qwen_resolved
                .env
                .get("HOME")
                .expect("HOME should be set for qwen endpoint"),
        );
        let qwen_settings =
            tokio::fs::read_to_string(qwen_home.join(".qwen").join("settings.json"))
                .await
                .expect("read qwen settings");
        assert!(qwen_settings.contains("\"selectedType\": \"openai\""));

        let opencode_endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_OPENCODE,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenCode custom".to_string(),
                base_url: Some(base_url.to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some(model_override.to_string()),
                api_key: Some("opencode-key".to_string()),
            },
        )
        .await
        .expect("upsert opencode endpoint");
        set_provider_source_selection(
            root.path(),
            PROVIDER_OPENCODE,
            HarnessSourceKind::Endpoint,
            Some(opencode_endpoint.id.clone()),
        )
        .await
        .expect("select opencode endpoint");
        let opencode_resolved = resolve_provider_source_for_run(root.path(), PROVIDER_OPENCODE)
            .await
            .expect("resolve opencode");
        let config = opencode_resolved
            .env
            .get("OPENCODE_CONFIG_CONTENT")
            .expect("opencode config env");
        let parsed: serde_json::Value = serde_json::from_str(config).expect("valid json");
        assert_eq!(
            parsed.get("model").and_then(serde_json::Value::as_str),
            Some("myawesomeprovider/openai/gpt-5.2-codex")
        );
        assert!(
            parsed
                .get("provider")
                .and_then(|provider| provider.get("myawesomeprovider"))
                .is_some(),
            "expected namespaced provider config key"
        );
        assert!(
            !opencode_resolved.env.contains_key("OPENROUTER_API_KEY"),
            "custom namespace should not force OPENROUTER_* compatibility env vars"
        );
    }

    #[tokio::test]
    async fn additional_provider_endpoint_env_projection_smoke() {
        let root = tempfile::tempdir().expect("tempdir");
        let cases: &[(&str, &[&str])] = &[
            (
                PROVIDER_QWEN,
                &["OPENAI_API_KEY", "OPENAI_BASE_URL", "HOME"],
            ),
            (
                PROVIDER_OPENCODE,
                &[
                    "OPENAI_API_KEY",
                    "OPENROUTER_API_KEY",
                    "OPENCODE_CONFIG_CONTENT",
                ],
            ),
            (
                PROVIDER_GOOSE,
                &[
                    "OPENAI_API_KEY",
                    "GOOSE_PROVIDER",
                    "GOOSE_DISABLE_KEYRING",
                    "GOOSE_MODEL",
                ],
            ),
            (PROVIDER_CAGENT, &["OPENAI_API_KEY"]),
            (PROVIDER_MISTRAL, &["MISTRAL_API_KEY", "MISTRAL_BASE_URL"]),
            (PROVIDER_AMP, &["AMP_API_KEY"]),
            (PROVIDER_DROID, &["FACTORY_API_KEY"]),
            (PROVIDER_CODY, &["SRC_ACCESS_TOKEN", "SRC_ENDPOINT"]),
            (PROVIDER_CONTINUE, &["CONTINUE_API_KEY"]),
            (PROVIDER_COPILOT, &["GH_TOKEN", "GITHUB_TOKEN"]),
            (
                PROVIDER_KIRO,
                &["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"],
            ),
            (
                PROVIDER_AUGGIE,
                &["AUGMENT_SESSION_AUTH", "AUGMENT_API_TOKEN"],
            ),
            (
                PROVIDER_PI,
                &["OPENAI_API_KEY", "PI_ACP_PROVIDER", "PI_ACP_MODEL"],
            ),
            (PROVIDER_CLINE, &["OPENAI_API_KEY", "CLINE_DIR", "HOME"]),
            (
                PROVIDER_OPENHANDS,
                &["LLM_API_KEY", "LLM_BASE_URL", "LLM_MODEL"],
            ),
        ];

        for (provider_id, required_keys) in cases {
            let endpoint = upsert_provider_endpoint(
                root.path(),
                provider_id,
                HarnessEndpointUpsert {
                    endpoint_id: None,
                    name: format!("{provider_id} endpoint"),
                    base_url: if *provider_id == PROVIDER_COPILOT
                        || *provider_id == PROVIDER_KIRO
                        || *provider_id == PROVIDER_AUGGIE
                        || *provider_id == PROVIDER_PI
                    {
                        None
                    } else {
                        Some("https://openrouter.ai/api/v1".to_string())
                    },
                    api_shape: if *provider_id == PROVIDER_COPILOT
                        || *provider_id == PROVIDER_KIRO
                        || *provider_id == PROVIDER_AUGGIE
                        || *provider_id == PROVIDER_PI
                    {
                        None
                    } else {
                        Some(HarnessApiShape::OpenaiResponses)
                    },
                    auth_type: None,
                    model_override: Some("test-model".to_string()),
                    api_key: Some(if *provider_id == PROVIDER_KIRO {
                        r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()
                    } else {
                        "test-key".to_string()
                    }),
                },
            )
            .await
            .expect("upsert endpoint");

            set_provider_source_selection(
                root.path(),
                provider_id,
                HarnessSourceKind::Endpoint,
                Some(endpoint.id.clone()),
            )
            .await
            .expect("select endpoint");

            let resolved = resolve_provider_source_for_run(root.path(), provider_id)
                .await
                .expect("resolve run");
            for key in *required_keys {
                assert!(
                    resolved.env.contains_key(*key),
                    "{provider_id} missing env key {key}"
                );
            }
        }
    }

    #[tokio::test]
    async fn copilot_endpoint_allows_token_only_upsert() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_COPILOT,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Copilot token".to_string(),
                base_url: None,
                api_shape: None,
                auth_type: None,
                model_override: None,
                api_key: Some("ghp_test".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        assert!(endpoint.base_url.is_none());
        assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

        let cfg = get_provider_source_config(root.path(), PROVIDER_COPILOT)
            .await
            .expect("get source config");
        let stored = cfg
            .endpoints
            .iter()
            .find(|candidate| candidate.id == endpoint.id)
            .expect("stored endpoint");
        assert!(stored.base_url.is_none());
        assert_eq!(stored.api_shape, HarnessApiShape::OpenaiResponses);
    }

    #[tokio::test]
    async fn pi_endpoint_allows_token_only_upsert_and_optional_base_url() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_PI,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Pi token".to_string(),
                base_url: None,
                api_shape: None,
                auth_type: None,
                model_override: Some("gpt-5".to_string()),
                api_key: Some("pi-key".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        assert!(endpoint.base_url.is_none());
        assert_eq!(endpoint.api_shape, HarnessApiShape::OpenaiResponses);

        set_provider_source_selection(
            root.path(),
            PROVIDER_PI,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run(root.path(), PROVIDER_PI)
            .await
            .expect("resolve run");
        assert_eq!(
            resolved.env.get("OPENAI_API_KEY"),
            Some(&"pi-key".to_string())
        );
        assert_eq!(
            resolved.env.get("PI_ACP_PROVIDER"),
            Some(&"openai".to_string())
        );
        assert_eq!(resolved.env.get("PI_ACP_MODEL"), Some(&"gpt-5".to_string()));
        assert!(!resolved.env.contains_key("OPENAI_BASE_URL"));
    }

    #[tokio::test]
    async fn kiro_endpoint_uses_runtime_root_for_run_resolution_when_provided() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime_root = tempfile::tempdir().expect("runtime tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIRO,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kiro token".to_string(),
                base_url: None,
                api_shape: None,
                auth_type: None,
                model_override: None,
                api_key: Some(r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIRO,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run_with_runtime_root(
            root.path(),
            PROVIDER_KIRO,
            Some(runtime_root.path()),
        )
        .await
        .expect("resolve run with runtime root");

        let home = PathBuf::from(
            resolved
                .env
                .get("HOME")
                .expect("HOME should be set for kiro endpoint"),
        );
        assert!(home.starts_with(runtime_root.path()));

        let token_path = home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH);
        assert!(token_path.exists());
    }

    #[tokio::test]
    async fn deleting_codex_endpoint_removes_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "OpenRouter".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: None,
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CODEX,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        resolve_provider_source_for_probe(root.path(), PROVIDER_CODEX)
            .await
            .expect("resolve probe");

        let endpoint_home = codex_endpoint_home(root.path(), &endpoint.id);
        assert!(endpoint_home.join("auth.json").exists());

        delete_provider_endpoint(root.path(), PROVIDER_CODEX, &endpoint.id)
            .await
            .expect("delete endpoint");

        assert!(!endpoint_home.exists());
    }

    #[tokio::test]
    async fn deleting_qwen_endpoint_removes_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_QWEN,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Qwen endpoint".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some("openai/gpt-5.2-codex".to_string()),
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert");

        set_provider_source_selection(
            root.path(),
            PROVIDER_QWEN,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select");

        resolve_provider_source_for_probe(root.path(), PROVIDER_QWEN)
            .await
            .expect("resolve probe");

        let endpoint_home = qwen_endpoint_home(root.path(), &endpoint.id);
        assert!(endpoint_home.join(".qwen").join("settings.json").exists());

        delete_provider_endpoint(root.path(), PROVIDER_QWEN, &endpoint.id)
            .await
            .expect("delete endpoint");

        assert!(!endpoint_home.exists());
    }

    #[tokio::test]
    async fn deleting_cline_endpoint_removes_runtime_root_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime_root = root
            .path()
            .join("containers")
            .join("workspaces")
            .join("workspace-cline")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root)
            .await
            .expect("runtime root");

        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CLINE,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Cline endpoint".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: Some("openai/gpt-5.2-codex".to_string()),
                api_key: Some("sk-test".to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_CLINE,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        let resolved = resolve_provider_source_for_run_with_runtime_root(
            root.path(),
            PROVIDER_CLINE,
            Some(&runtime_root),
        )
        .await
        .expect("resolve run with runtime root");

        let endpoint_home = cline_endpoint_home(&runtime_root, &endpoint.id);
        assert!(endpoint_home.exists());
        assert_eq!(
            resolved.env.get("CLINE_DIR"),
            Some(&endpoint_home.to_string_lossy().to_string())
        );
        assert_eq!(
            resolved.env.get("HOME"),
            Some(&endpoint_home.to_string_lossy().to_string())
        );

        delete_provider_endpoint(root.path(), PROVIDER_CLINE, &endpoint.id)
            .await
            .expect("delete endpoint");

        assert!(!endpoint_home.exists());
    }

    #[tokio::test]
    async fn deleting_kiro_endpoint_removes_runtime_root_endpoint_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime_root = root
            .path()
            .join("containers")
            .join("workspaces")
            .join("workspace-kiro")
            .join("data");
        tokio::fs::create_dir_all(&runtime_root)
            .await
            .expect("runtime root");

        let endpoint = upsert_provider_endpoint(
            root.path(),
            PROVIDER_KIRO,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "Kiro token".to_string(),
                base_url: None,
                api_shape: None,
                auth_type: None,
                model_override: None,
                api_key: Some(r#"{"token":"test-token","exp":"2099-01-01T00:00:00Z"}"#.to_string()),
            },
        )
        .await
        .expect("upsert endpoint");

        set_provider_source_selection(
            root.path(),
            PROVIDER_KIRO,
            HarnessSourceKind::Endpoint,
            Some(endpoint.id.clone()),
        )
        .await
        .expect("select endpoint");

        resolve_provider_source_for_run_with_runtime_root(
            root.path(),
            PROVIDER_KIRO,
            Some(&runtime_root),
        )
        .await
        .expect("resolve run with runtime root");

        let endpoint_home = kiro_endpoint_home(&runtime_root, &endpoint.id);
        assert!(endpoint_home.join(KIRO_AUTH_TOKEN_RELATIVE_PATH).exists());

        delete_provider_endpoint(root.path(), PROVIDER_KIRO, &endpoint.id)
            .await
            .expect("delete endpoint");

        assert!(!endpoint_home.exists());
    }

    #[tokio::test]
    async fn shape_compatibility_rejects_mismatch() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: None,
                name: "wrong".to_string(),
                base_url: Some("https://example.com".to_string()),
                api_shape: Some(HarnessApiShape::AnthropicMessages),
                auth_type: None,
                model_override: None,
                api_key: Some("k".to_string()),
            },
        )
        .await
        .expect_err("expected shape mismatch");
        assert!(err
            .to_string()
            .contains("codex requires api_shape=openai_responses"));
    }

    #[tokio::test]
    async fn unsafe_endpoint_id_is_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = upsert_provider_endpoint(
            root.path(),
            PROVIDER_CODEX,
            HarnessEndpointUpsert {
                endpoint_id: Some("../escape".to_string()),
                name: "bad".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: Some(HarnessApiShape::OpenaiResponses),
                auth_type: None,
                model_override: None,
                api_key: Some("k".to_string()),
            },
        )
        .await
        .expect_err("unsafe endpoint id should fail");
        assert!(err
            .to_string()
            .contains("endpoint_id may only contain ASCII letters"));
    }
}
